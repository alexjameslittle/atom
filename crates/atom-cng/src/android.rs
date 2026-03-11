use atom_ffi::AtomResult;
use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, metadata_target};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;

use crate::templates::render;
use crate::{GenerationPlan, PlatformPlan};

pub(crate) fn build_android_plan(
    app: &AppConfig,
    build: &BuildConfig,
    android: &AndroidConfig,
) -> PlatformPlan {
    let generated_root = build.generated_root.join("android").join(&app.slug);
    let package_dir = kotlin_package_dir(
        android
            .application_id
            .as_deref()
            .expect("android application id should be present when enabled"),
    );
    let source_root = generated_root.join("src/main/kotlin").join(package_dir);
    let files = vec![
        generated_root.join("BUILD.bazel"),
        generated_root.join("AndroidManifest.generated.xml"),
        generated_root.join("atom_runtime_jni.rs"),
        source_root.join("AtomApplication.kt"),
        source_root.join("AtomBindings.kt"),
        source_root.join("MainActivity.kt"),
    ];
    PlatformPlan {
        target: format!("//{}:app", generated_root.as_str()),
        generated_root,
        files,
    }
}

pub(crate) fn render_android_build_file(
    app: &AppConfig,
    modules: &[ResolvedModule],
    android: &AndroidConfig,
    resource_files: &[String],
) -> AtomResult<String> {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    let package_dir = kotlin_package_dir(package_name);
    let source_root = Utf8PathBuf::from("src/main/kotlin").join(package_dir);
    let module_labels: Vec<String> = modules
        .iter()
        .map(|m| metadata_target(&m.request.target_label, "_android_srcs"))
        .collect::<AtomResult<_>>()?;
    render(
        "android/BUILD.bazel",
        context! {
            jni_prefix => jni_prefix(package_name),
            entry_crate_label => &app.entry_crate_label,
            source_root => source_root.as_str(),
            module_labels,
            package_name,
            resource_files,
            target_sdk => android.target_sdk.unwrap_or_default(),
        },
    )
}

pub(crate) fn render_android_manifest_xml(
    android: &AndroidConfig,
    manifest: &JsonMap,
) -> AtomResult<String> {
    crate::render_android_manifest_document(
        android.application_id.as_deref().unwrap_or_default(),
        manifest,
    )
}

pub(crate) fn render_kotlin_application(
    app: &AppConfig,
    android: &AndroidConfig,
) -> AtomResult<String> {
    render(
        "android/AtomApplication.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            name => &app.name,
            slug => &app.slug,
        },
    )
}

pub(crate) fn render_kotlin_bindings(
    modules: &[ResolvedModule],
    android: &AndroidConfig,
) -> AtomResult<String> {
    let module_ids: Vec<&str> = modules.iter().map(|m| m.manifest.id.as_str()).collect();
    render(
        "android/AtomBindings.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            module_ids,
        },
    )
}

pub(crate) fn render_kotlin_main_activity(
    app: &AppConfig,
    _generated_root: &Utf8Path,
    android: &AndroidConfig,
) -> AtomResult<String> {
    render(
        "android/MainActivity.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            name => &app.name,
            slug => &app.slug,
        },
    )
}

pub(crate) fn render_android_runtime_jni(
    app: &AppConfig,
    android: &AndroidConfig,
) -> AtomResult<String> {
    render(
        "android/atom_runtime_jni.rs",
        context! {
            entry_crate_name => &app.entry_crate_name,
            jni_prefix => jni_prefix(android.application_id.as_deref().unwrap_or_default()),
        },
    )
}

pub(crate) fn kotlin_package_dir(application_id: &str) -> Utf8PathBuf {
    Utf8PathBuf::from(application_id.replace('.', "/"))
}

fn jni_prefix(application_id: &str) -> String {
    format!(
        "Java_{}_AtomRuntimeBridge",
        jni_mangle_segment(application_id)
    )
}

fn jni_mangle_segment(value: &str) -> String {
    let mut mangled = String::new();
    for character in value.chars() {
        match character {
            '.' => mangled.push('_'),
            '_' => mangled.push_str("_1"),
            '$' => mangled.push_str("_00024"),
            value if value.is_ascii_alphanumeric() => mangled.push(value),
            _ => mangled.push('_'),
        }
    }
    mangled
}

pub(crate) fn emit_android_host_tree(
    repo_root: &Utf8Path,
    plan: &GenerationPlan,
) -> AtomResult<()> {
    let Some(android) = &plan.android else {
        return Ok(());
    };
    let android_config = plan
        .android_config
        .as_ref()
        .expect("android config should exist when android output exists");
    crate::emit::write_file(
        &repo_root.join(&android.generated_root).join("BUILD.bazel"),
        &render_android_build_file(
            &plan.app,
            &plan.modules,
            android_config,
            &plan.android_resources,
        )?,
    )?;
    crate::emit::write_file(
        &repo_root
            .join(&android.generated_root)
            .join("AndroidManifest.generated.xml"),
        &render_android_manifest_xml(android_config, &plan.android_manifest)?,
    )?;
    crate::emit::write_file(
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
    crate::emit::write_file(
        &repo_root.join(&package_dir).join("AtomApplication.kt"),
        &render_kotlin_application(&plan.app, android_config)?,
    )?;
    crate::emit::write_file(
        &repo_root.join(&package_dir).join("AtomBindings.kt"),
        &render_kotlin_bindings(&plan.modules, android_config)?,
    )?;
    crate::emit::write_file(
        &repo_root.join(&package_dir).join("MainActivity.kt"),
        &render_kotlin_main_activity(&plan.app, &android.generated_root, android_config)?,
    )?;
    Ok(())
}
