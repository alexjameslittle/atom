use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::devices::android::resolve_android_device;
use crate::devices::ios::{IosDestinationKind, prepare_ios_simulator, resolve_ios_destination};
use crate::tools::{
    ToolRunner, find_bazel_output, find_bazel_output_owned, run_bazel, run_bazel_owned, run_tool,
};

pub(crate) fn deploy_ios(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let destination = resolve_ios_destination(repo_root, runner, requested_device)?;
    let target = generated_target(manifest, "ios");
    let build_args = ios_bazel_args(&target, destination.kind);
    run_bazel_owned(runner, repo_root, &build_args)?;

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
        IosDestinationKind::Simulator => {
            let simulator = prepare_ios_simulator(repo_root, runner, &destination)?;
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &["simctl", "install", &simulator, installable_app.as_str()],
            )?;
            run_tool(
                runner,
                repo_root,
                "xcrun",
                &["simctl", "launch", &simulator, bundle_id],
            )?;
        }
        IosDestinationKind::Device => {
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
                    &destination.id,
                    installable_app.as_str(),
                ],
            )?;
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
                    &destination.id,
                    bundle_id,
                ],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn deploy_android(
    repo_root: &Utf8Path,
    manifest: &NormalizedManifest,
    requested_device: Option<&str>,
    runner: &mut impl ToolRunner,
) -> AtomResult<()> {
    let target = generated_target(manifest, "android");
    run_bazel(runner, repo_root, &["build", &target])?;

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
        run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "install", "-r", apk.as_str()],
        )?;
        run_tool(
            runner,
            repo_root,
            "adb",
            &["-s", serial, "shell", "am", "start", "-n", &component],
        )?;
    } else {
        run_tool(runner, repo_root, "adb", &["install", "-r", apk.as_str()])?;
        run_tool(
            runner,
            repo_root,
            "adb",
            &["shell", "am", "start", "-n", &component],
        )?;
    }
    Ok(())
}

pub(crate) fn generated_target(manifest: &NormalizedManifest, platform: &str) -> String {
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

    find_descendant_with_suffix(
        path.parent().ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ExternalToolFailed,
                "bazelisk returned an invalid iOS artifact path",
                path.as_str(),
            )
        })?,
        ".app",
    )?
    .ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::ExternalToolFailed,
            "could not locate an unpacked .app bundle next to the built .ipa",
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
