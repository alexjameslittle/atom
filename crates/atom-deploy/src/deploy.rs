use std::fs;
use std::process::Command;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::devices::android::resolve_android_device;
use crate::devices::ios::{IosDestinationKind, prepare_ios_simulator, resolve_ios_destination};
use crate::progress::run_step;
use crate::tools::{
    ToolRunner, find_bazel_output, find_bazel_output_owned, run_bazel, run_bazel_owned, run_tool,
    stream_tool,
};

/// # Errors
///
/// Returns an error if device resolution, building, or installation fails.
pub fn deploy_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_device)?;
    let target = generated_target(manifest, "ios");
    let build_args = ios_bazel_args(&target, destination.kind);

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
            runner,
            &destination,
            &installable_app,
            bundle_id,
        ),
        IosDestinationKind::Device => install_and_launch_device(
            repo_root,
            runner,
            &destination.id,
            &installable_app,
            bundle_id,
        ),
    }
}

fn install_and_launch_simulator(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    destination: &crate::devices::ios::IosDestination,
    installable_app: &Utf8Path,
    bundle_id: &str,
) -> AtomResult<()> {
    let simulator = run_step(
        "Preparing simulator...",
        "Simulator ready",
        "Simulator preparation failed",
        || prepare_ios_simulator(repo_root, runner, destination),
    )?;
    run_step(
        "Installing app...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &["simctl", "install", &simulator, installable_app.as_str()],
            )
        },
    )?;
    eprintln!("→ Launching app and streaming logs... (Ctrl+C to stop)");
    stream_tool(
        runner,
        repo_root,
        "xcrun",
        &["simctl", "launch", "--console", &simulator, bundle_id],
    )
}

fn install_and_launch_device(
    repo_root: &Utf8Path,
    runner: &mut impl ToolRunner,
    device_id: &str,
    installable_app: &Utf8Path,
    bundle_id: &str,
) -> AtomResult<()> {
    run_step(
        "Installing app on device...",
        "App installed",
        "Installation failed",
        || {
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &[
                    "devicectl",
                    "device",
                    "install",
                    "app",
                    "--device",
                    device_id,
                    installable_app.as_str(),
                ],
            )
        },
    )?;
    run_step("Launching app...", "App launched", "Launch failed", || {
        run_tool(
            runner,
            repo_root,
            "xcrun",
            &[
                "devicectl",
                "device",
                "process",
                "launch",
                "--device",
                device_id,
                bundle_id,
            ],
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
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let target = generated_target(manifest, "android");

    run_step(
        "Building Android app...",
        "Built Android app",
        "Android build failed",
        || run_bazel(runner, repo_root, &["build", &target]),
    )?;

    let apk = find_bazel_output(runner, repo_root, &target, &["app.apk", ".apk"], "APK")?;
    let application_id = manifest.android.application_id.as_deref().ok_or_else(|| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            "validated Android manifest is missing application_id",
        )
    })?;

    let selected_serial = resolve_android_device(repo_root, runner, requested_device)?;
    let component = format!("{application_id}/.MainActivity");
    if let Some(serial) = selected_serial.as_deref() {
        run_step(
            "Installing app...",
            "App installed",
            "Installation failed",
            || {
                run_tool(
                    runner,
                    repo_root,
                    "adb",
                    &["-s", serial, "install", "-r", apk.as_str()],
                )
            },
        )?;
        run_step("Launching app...", "App launched", "Launch failed", || {
            run_tool(
                runner,
                repo_root,
                "adb",
                &["-s", serial, "shell", "am", "start", "-n", &component],
            )
        })?;
    } else {
        run_step(
            "Installing app...",
            "App installed",
            "Installation failed",
            || run_tool(runner, repo_root, "adb", &["install", "-r", apk.as_str()]),
        )?;
        run_step("Launching app...", "App launched", "Launch failed", || {
            run_tool(
                runner,
                repo_root,
                "adb",
                &["shell", "am", "start", "-n", &component],
            )
        })?;
    }
    Ok(())
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

fn ios_bazel_args(target: &str, destination: IosDestinationKind) -> Vec<String> {
    let cpu = match destination {
        IosDestinationKind::Simulator => "sim_arm64",
        IosDestinationKind::Device => "arm64",
    };
    vec![
        "build".to_owned(),
        target.to_owned(),
        format!("--ios_multi_cpus={cpu}"),
    ]
}

fn resolve_ios_installable_artifact(path: &Utf8Path) -> AtomResult<Utf8PathBuf> {
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
