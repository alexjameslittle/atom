use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

use crate::android::{
    kotlin_package_dir, render_android_build_file, render_android_manifest_xml,
    render_android_runtime_jni, render_kotlin_application, render_kotlin_bindings,
    render_kotlin_main_activity,
};
use crate::ios::{
    render_ios_build_file, render_ios_launch_storyboard, render_ios_plist,
    render_ios_runtime_bridge, render_ios_runtime_header, render_swift_app_delegate,
    render_swift_bindings, render_swift_main, render_swift_scene_delegate,
};
use crate::{ContributedFile, FileSource, GenerationPlan};

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
    copy_schema_files(repo_root, plan)?;
    copy_contributed_files(repo_root, &plan.contributed_files)?;
    emit_ios_host_tree(repo_root, plan)?;
    emit_android_host_tree(repo_root, plan)?;
    Ok(generated_roots(plan))
}

fn copy_schema_files(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
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
    Ok(())
}

fn emit_ios_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    if let Some(ios) = &plan.ios {
        let ios_config = plan
            .ios_config
            .as_ref()
            .expect("ios config should exist when ios output exists");
        write_file(
            &repo_root.join(&ios.generated_root).join("BUILD.bazel"),
            &render_ios_build_file(
                &plan.app,
                &plan.modules,
                ios_config,
                &plan.ios_resources,
                &plan.ios_resource_globs,
            )?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("Info.generated.plist"),
            &render_ios_plist(&plan.plist)?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("LaunchScreen.storyboard"),
            &render_ios_launch_storyboard(),
        )?;
        write_file(
            &repo_root.join(&ios.generated_root).join("atom_runtime.h"),
            &render_ios_runtime_header(),
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("atom_runtime_app_bridge.rs"),
            &render_ios_runtime_bridge(&plan.app)?,
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
    Ok(())
}

fn emit_android_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    if let Some(android) = &plan.android {
        let android_config = plan
            .android_config
            .as_ref()
            .expect("android config should exist when android output exists");
        write_file(
            &repo_root.join(&android.generated_root).join("BUILD.bazel"),
            &render_android_build_file(
                &plan.app,
                &plan.modules,
                android_config,
                &plan.android_resources,
            )?,
        )?;
        write_file(
            &repo_root
                .join(&android.generated_root)
                .join("AndroidManifest.generated.xml"),
            &render_android_manifest_xml(android_config, &plan.android_manifest)?,
        )?;
        write_file(
            &repo_root
                .join(&android.generated_root)
                .join("atom_runtime_jni.rs"),
            &render_android_runtime_jni(&plan.app, android_config)?,
        )?;
        let package_dir = android
            .generated_root
            .join("src/main/kotlin")
            .join(kotlin_package_dir(
                android_config
                    .application_id
                    .as_deref()
                    .expect("android application id should exist when enabled"),
            ));
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
    Ok(())
}

fn copy_contributed_files(repo_root: &Utf8Path, files: &[ContributedFile]) -> AtomResult<()> {
    for file in files {
        let destination = repo_root.join(&file.output);
        match &file.source {
            FileSource::Copy(source) => copy_path(&repo_root.join(source), &destination)?,
            FileSource::Content(contents) => write_file(&destination, contents)?,
        }
    }
    Ok(())
}

fn generated_roots(plan: &GenerationPlan) -> Vec<Utf8PathBuf> {
    let mut roots = Vec::new();
    if let Some(ios) = &plan.ios {
        roots.push(ios.generated_root.clone());
    }
    if let Some(android) = &plan.android {
        roots.push(android.generated_root.clone());
    }
    roots
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

fn copy_path(source: &Utf8Path, destination: &Utf8Path) -> AtomResult<()> {
    let metadata = fs::metadata(source).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to stat contributed source: {error}"),
            source.as_str(),
        )
    })?;

    if metadata.is_dir() {
        fs::create_dir_all(destination).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to create contributed directory: {error}"),
                destination.as_str(),
            )
        })?;
        for entry in fs::read_dir(source).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::CngWriteError,
                format!("failed to read contributed directory: {error}"),
                source.as_str(),
            )
        })? {
            let entry = entry.map_err(|error| {
                AtomError::with_path(
                    AtomErrorCode::CngWriteError,
                    format!("failed to read contributed directory entry: {error}"),
                    source.as_str(),
                )
            })?;
            let entry_path = Utf8PathBuf::from_path_buf(entry.path()).map_err(|_| {
                AtomError::with_path(
                    AtomErrorCode::CngWriteError,
                    "contributed directory entry path must be valid UTF-8",
                    source.as_str(),
                )
            })?;
            let destination_entry = destination.join(entry.file_name().to_string_lossy().as_ref());
            copy_path(&entry_path, &destination_entry)?;
        }
        return Ok(());
    }

    write_parent_dir(destination)?;
    fs::copy(source, destination).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngWriteError,
            format!("failed to copy contributed file: {error}"),
            destination.as_str(),
        )
    })?;
    Ok(())
}
