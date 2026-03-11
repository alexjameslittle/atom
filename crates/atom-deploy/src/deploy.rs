use std::fs;
use std::process::Command;
use std::thread;
use std::time::Duration;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::backends::{DeployBackendRegistry, first_party_deploy_backend_registry};
use crate::devices::android::{prepare_android_emulator, resolve_android_device};
use crate::devices::ios::{
    IosDestination, IosDestinationKind, prepare_ios_simulator, resolve_ios_destination,
};
use crate::evaluate::wait_for_ios_launch_ready;
use crate::progress::run_step;
use crate::tools::{
    ToolRunner, capture_tool, find_bazel_output_owned, run_bazel_owned, run_tool, stream_tool,
};

const ANDROID_APP_PID_WAIT_ATTEMPTS: usize = 30;
const ANDROID_APP_PID_WAIT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Attached,
    Detached,
}

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or deployment fails.
pub fn deploy_backend(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    backend_id: &str,
    requested_device: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let registry = first_party_deploy_backend_registry();
    deploy_backend_with_registry(
        repo_root,
        manifest,
        &registry,
        backend_id,
        requested_device,
        launch_mode,
        runner,
    )
}

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or deployment fails.
pub fn deploy_backend_with_registry(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    requested_device: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let backend = resolve_backend(registry, backend_id)?;
    require_enabled_backend(backend.platform(), backend.is_enabled(manifest))?;
    backend.deploy(repo_root, manifest, requested_device, launch_mode, runner)
}

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or stopping fails.
pub fn stop_backend(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    backend_id: &str,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let registry = first_party_deploy_backend_registry();
    stop_backend_with_registry(
        repo_root,
        manifest,
        &registry,
        backend_id,
        requested_device,
        runner,
    )
}

/// # Errors
///
/// Returns an error if the backend id is unknown, disabled for the app, or stopping fails.
pub fn stop_backend_with_registry(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    registry: &DeployBackendRegistry,
    backend_id: &str,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let backend = resolve_backend(registry, backend_id)?;
    require_enabled_backend(backend.platform(), backend.is_enabled(manifest))?;
    backend.stop(repo_root, manifest, requested_device, runner)
}

fn resolve_backend<'a>(
    registry: &'a DeployBackendRegistry,
    backend_id: &str,
) -> AtomResult<&'a dyn crate::backends::DeployBackend> {
    registry.get(backend_id).map(Box::as_ref).ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            format!("unknown backend id: {backend_id}"),
            backend_id,
        )
    })
}

fn require_enabled_backend(platform: &str, enabled: bool) -> AtomResult<()> {
    if enabled {
        Ok(())
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("{platform} platform is not enabled"),
            platform,
        ))
    }
}

/// # Errors
///
/// Returns an error if device resolution, building, or installation fails.
pub fn deploy_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_device)?;
    let target = generated_target(manifest, "ios");
    let build_args = ios_bazel_args(&target, &destination);

    run_step(
        "Building iOS app...",
        "Built iOS app",
        "iOS build failed",
        || run_bazel_owned(runner, repo_root, &build_args),
    )?;

    let app_bundle = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &[".app", ".ipa"],
        "iOS app artifact",
    )?;
    let installable_app = resolve_ios_installable_artifact(&app_bundle)?;
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;

    match destination.kind {
        IosDestinationKind::Simulator => install_and_launch_simulator(
            repo_root,
            manifest,
            runner,
            &destination,
            &installable_app,
            bundle_id,
            launch_mode,
        ),
        IosDestinationKind::Device => install_and_launch_device(
            repo_root,
            manifest,
            runner,
            &destination.id,
            &installable_app,
            bundle_id,
            launch_mode,
        ),
    }
}

fn install_and_launch_simulator(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut (impl ToolRunner + ?Sized),
    destination: &crate::devices::ios::IosDestination,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    let target_id = run_step(
        "Preparing simulator...",
        "Simulator ready",
        "Simulator preparation failed",
        || prepare_ios_simulator(repo_root, runner, destination),
    )?;
    install_and_launch_with_idb(
        repo_root,
        manifest,
        runner,
        &target_id,
        installable_app,
        bundle_id,
        launch_mode,
    )
}

fn install_and_launch_with_idb(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut (impl ToolRunner + ?Sized),
    destination_id: &str,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    run_step(
        "Installing app...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "idb",
                &[
                    "install",
                    "--udid",
                    destination_id,
                    installable_app.as_str(),
                ],
            )
        },
    )?;
    let _ = run_tool(
        runner,
        repo_root,
        "idb",
        &["terminate", "--udid", destination_id, bundle_id],
    );
    match launch_mode {
        LaunchMode::Attached => {
            eprintln!("→ Launching app and streaming logs... (Ctrl+C to stop)");
            stream_tool(
                runner,
                repo_root,
                "idb",
                &["launch", "-f", "-w", "--udid", destination_id, bundle_id],
            )
        }
        LaunchMode::Detached => {
            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "idb",
                    &["launch", "-f", "--udid", destination_id, bundle_id],
                )?;
                wait_for_ios_launch_ready(repo_root, manifest, destination_id, runner)
            })
        }
    }
}

fn install_and_launch_device(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    runner: &mut (impl ToolRunner + ?Sized),
    device_id: &str,
    installable_app: &Utf8Path,
    bundle_id: &str,
    launch_mode: LaunchMode,
) -> AtomResult<()> {
    install_and_launch_with_idb(
        repo_root,
        manifest,
        runner,
        device_id,
        installable_app,
        bundle_id,
        launch_mode,
    )
}

/// # Errors
///
/// Returns an error if destination resolution or stop coordination fails.
pub fn stop_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_device)?;
    let bundle_id = manifest.ios.bundle_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated iOS manifest is missing bundle_id",
        )
    })?;

    if destination.kind == IosDestinationKind::Simulator && !destination.is_booted_simulator() {
        return Ok(());
    }

    if !ios_app_is_running(repo_root, runner, &destination.id, bundle_id)? {
        return Ok(());
    }

    run_step("Stopping app...", "App stopped", "Stop failed", || {
        run_tool(
            runner,
            repo_root,
            "idb",
            &["terminate", "--udid", &destination.id, bundle_id],
        )
    })
}

/// # Errors
///
/// Returns an error if device resolution, building, or installation fails.
pub fn deploy_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    launch_mode: LaunchMode,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    let destination = resolve_android_device(repo_root, runner, requested_device)?;
    let target = generated_target(manifest, "android");
    let build_args = vec![
        "build".to_owned(),
        target.clone(),
        "--android_platforms=//platforms:arm64-v8a".to_owned(),
    ];

    run_step(
        "Building Android app...",
        "Built Android app",
        "Android build failed",
        || run_bazel_owned(runner, repo_root, &build_args),
    )?;

    let apk = find_bazel_output_owned(
        runner,
        repo_root,
        &build_args,
        &target,
        &["app.apk", ".apk"],
        "APK",
    )?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    let serial = run_step(
        "Preparing emulator...",
        "Emulator ready",
        "Emulator preparation failed",
        || prepare_android_emulator(repo_root, runner, &destination),
    )?;

    let component = format!("{application_id}/.MainActivity");
    run_step(
        "Installing app...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", &serial, "install", "-r", apk.as_str()],
            )
        },
    )?;
    match launch_mode {
        LaunchMode::Attached => {
            // Clear logcat before launch so we capture all logs from app start.
            run_tool(runner, repo_root, "adb", &["-s", &serial, "logcat", "-c"])?;

            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &[
                        "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
                    ],
                )
            })?;

            // Wait for the app process to appear, then stream only its logs.
            let pid = wait_for_app_pid(runner, repo_root, &serial, application_id)?;

            eprintln!("→ Streaming logs for {application_id} (pid {pid})... (Ctrl+C to stop)");
            stream_tool(
                runner,
                repo_root,
                "adb",
                &[
                    "-s",
                    &serial,
                    "logcat",
                    "--pid",
                    &pid,
                    "-s",
                    "AtomRuntime:*",
                ],
            )
        }
        LaunchMode::Detached => {
            run_step("Launching app...", "App launched", "Launch failed", || {
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &[
                        "-s", &serial, "shell", "am", "start", "-W", "-n", &component,
                    ],
                )
            })
        }
    }
}

/// # Errors
///
/// Returns an error if destination resolution or stop coordination fails.
pub fn stop_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut (impl ToolRunner + ?Sized),
) -> AtomResult<()> {
    let destination = resolve_android_device(repo_root, runner, requested_device)?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    if destination.state == "avd" {
        return Ok(());
    }

    run_step("Stopping app...", "App stopped", "Stop failed", || {
        run_tool(
            runner,
            repo_root,
            "adb",
            &[
                "-s",
                &destination.serial,
                "shell",
                "am",
                "force-stop",
                application_id,
            ],
        )
    })
}

fn ios_app_is_running(
    repo_root: &Utf8Path,
    runner: &mut (impl ToolRunner + ?Sized),
    destination_id: &str,
    bundle_id: &str,
) -> AtomResult<bool> {
    let output = capture_tool(
        runner,
        repo_root,
        "idb",
        &["list-apps", "--udid", destination_id],
    )?;
    Ok(output.lines().any(|line| {
        let mut fields = line.split('|').map(str::trim);
        let identifier = fields.next();
        let _name = fields.next();
        let _install_type = fields.next();
        let _architectures = fields.next();
        let debug_state = fields.next();
        identifier == Some(bundle_id) && debug_state == Some("Running")
    }))
}

pub(crate) fn wait_for_app_pid(
    runner: &mut (impl ToolRunner + ?Sized),
    repo_root: &Utf8Path,
    serial: &str,
    application_id: &str,
) -> AtomResult<String> {
    for _ in 0..ANDROID_APP_PID_WAIT_ATTEMPTS {
        if let Ok(output) = capture_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "pidof", application_id],
        ) {
            let pid = output.trim();
            if !pid.is_empty() {
                return Ok(pid.to_owned());
            }
        }
        thread::sleep(ANDROID_APP_PID_WAIT_INTERVAL);
    }
    Err(AtomError::new(
        AtomErrorCode::ExternalToolFailed,
        format!(
            "could not find running process for {application_id} — the app may have crashed on launch"
        ),
    ))
}

#[must_use]
pub fn generated_target(manifest: &NormalizedManifest, platform: &str) -> String {
    format!(
        "//{}/{}/{}:app",
        manifest.build.generated_root.as_str(),
        platform,
        manifest.app.slug
    )
}

pub(crate) fn ios_bazel_args(target: &str, destination: &IosDestination) -> Vec<String> {
    let cpu = match destination.kind {
        IosDestinationKind::Simulator => "sim_arm64,x86_64",
        IosDestinationKind::Device => "arm64",
    };
    vec![
        "build".to_owned(),
        target.to_owned(),
        format!("--ios_multi_cpus={cpu}"),
    ]
}

pub(crate) fn resolve_ios_installable_artifact(path: &Utf8Path) -> AtomResult<Utf8PathBuf> {
    if path.extension() == Some("app") {
        return Ok(path.to_owned());
    }
    if path.extension() != Some("ipa") {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk did not produce an installable iOS artifact",
            path.as_str(),
        ));
    }

    let parent = path.parent().ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "bazelisk returned an invalid iOS artifact path",
            path.as_str(),
        )
    })?;

    // Check for an already-unpacked .app bundle next to the .ipa.
    if let Some(app) = find_descendant_with_suffix(parent, ".app")? {
        return Ok(app);
    }

    // Bazel may only produce the .ipa archive — unzip it to extract the .app.
    let extract_dir = parent.join("_ipa_extract");
    let _ = fs::remove_dir_all(&extract_dir);
    let status = Command::new("unzip")
        .args(["-q", "-o", path.as_str(), "-d", extract_dir.as_str()])
        .status()
        .map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to unzip .ipa: {error}"),
                path.as_str(),
            )
        })?;
    if !status.success() {
        return Err(AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "failed to unzip .ipa archive",
            path.as_str(),
        ));
    }

    find_descendant_with_suffix(&extract_dir, ".app")?.ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "unzipped .ipa did not contain a .app bundle",
            path.as_str(),
        )
    })
}

fn find_descendant_with_suffix(root: &Utf8Path, suffix: &str) -> AtomResult<Option<Utf8PathBuf>> {
    for entry in fs::read_dir(root).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            format!("failed to inspect generated iOS outputs: {error}"),
            root.as_str(),
        )
    })? {
        let entry = entry.map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to inspect generated iOS outputs: {error}"),
                root.as_str(),
            )
        })?;
        let path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                "generated iOS output path was not valid UTF-8",
                root.as_str(),
            )
        })?;
        if path.as_str().ends_with(suffix) {
            return Ok(Some(path));
        }
        if path.is_dir()
            && let Some(found) = find_descendant_with_suffix(&path, suffix)?
        {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
