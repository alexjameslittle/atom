use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

use crate::GenerationPlan;
use crate::android::{
    render_android_build_file, render_android_manifest_xml, render_kotlin_application,
    render_kotlin_bindings, render_kotlin_main_activity,
};
use crate::ios::{
    render_ios_build_file, render_ios_launch_storyboard, render_ios_plist,
    render_swift_app_delegate, render_swift_bindings, render_swift_main,
    render_swift_scene_delegate,
};

/// # Errors
///
/// Returns an error if any generated file or directory cannot be written.
///
/// # Panics
///
/// Panics if platform configs are missing when the corresponding platform plan
/// exists, or if schema files lack the expected generated prefix.
pub fn emit_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<Vec<Utf8PathBuf>> {
    write_file(
        &repo_root.join(&plan.schema.aggregate),
        &crate::render_aggregate_schema(plan)?,
    )?;

    for schema_file in &plan.schema_files {
        let destination = repo_root.join(&schema_file.output);
        write_parent_dir(&destination)?;
        fs::copy(&schema_file.source, &destination).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to copy schema file: {error}"),
                destination.as_str(),
            )
        })?;
    }

    if let Some(ios) = &plan.ios {
        let ios_config = plan
            .ios_config
            .as_ref()
            .expect("ios config should exist when ios output exists");
        write_file(
            &repo_root.join(&ios.generated_root).join("BUILD.bazel"),
            &render_ios_build_file(&plan.app, &plan.modules, ios_config)?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("Info.generated.plist"),
            &render_ios_plist(&plan.app, ios_config)?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("LaunchScreen.storyboard"),
            &render_ios_launch_storyboard(),
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("AtomAppDelegate.swift"),
            &render_swift_app_delegate(&plan.app),
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("SceneDelegate.swift"),
            &render_swift_scene_delegate(&plan.app)?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("AtomBindings.swift"),
            &render_swift_bindings(&plan.modules)?,
        )?;
        write_file(
            &repo_root.join(&ios.generated_root).join("main.swift"),
            &render_swift_main(),
        )?;
    }

    if let Some(android) = &plan.android {
        let android_config = plan
            .android_config
            .as_ref()
            .expect("android config should exist when android output exists");
        write_file(
            &repo_root.join(&android.generated_root).join("BUILD.bazel"),
            &render_android_build_file(&plan.app, &plan.modules, android_config)?,
        )?;
        write_file(
            &repo_root
                .join(&android.generated_root)
                .join("AndroidManifest.generated.xml"),
            &render_android_manifest_xml(&plan.app, android_config)?,
        )?;
        let package_dir = android.files[2]
            .parent()
            .expect("android application file parent")
            .to_owned();
        write_file(
            &repo_root.join(&package_dir).join("AtomApplication.kt"),
            &render_kotlin_application(&plan.app, android_config)?,
        )?;
        write_file(
            &repo_root.join(&package_dir).join("AtomBindings.kt"),
            &render_kotlin_bindings(&plan.modules, android_config)?,
        )?;
        write_file(
            &repo_root.join(&package_dir).join("MainActivity.kt"),
            &render_kotlin_main_activity(&plan.app, &android.generated_root, android_config)?,
        )?;
    }

    let mut roots = Vec::new();
    if let Some(ios) = &plan.ios {
        roots.push(ios.generated_root.clone());
    }
    if let Some(android) = &plan.android {
        roots.push(android.generated_root.clone());
    }
    Ok(roots)
}

fn write_file(path: &Utf8Path, contents: &str) -> AtomResult<()> {
    write_parent_dir(path)?;
    fs::write(path, contents).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to write generated file: {error}"),
            path.as_str(),
        )
    })
}

fn write_parent_dir(path: &Utf8Path) -> AtomResult<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to create parent directory: {error}"),
            parent.as_str(),
        )
    })
}
