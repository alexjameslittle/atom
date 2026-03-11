use atom_backends::{
    BackendDefinition, GenerationBackend, GenerationBackendRegistry, GenerationPlan, PlatformPlan,
};
use atom_cng::{render_android_manifest_document, render_template, write_generated_file};
use atom_ffi::AtomResult;
use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, NormalizedManifest, metadata_target};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;

const BACKEND_ID: &str = "android";

struct AndroidGenerationBackend;

pub fn register(registry: &mut GenerationBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(AndroidGenerationBackend))
}

impl BackendDefinition for AndroidGenerationBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "android"
    }
}

impl GenerationBackend for AndroidGenerationBackend {
    fn build_platform_plan(&self, manifest: &NormalizedManifest) -> Option<PlatformPlan> {
        manifest
            .android
            .enabled
            .then(|| build_android_plan(&manifest.app, &manifest.build, &manifest.android))
    }

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
        emit_android_host_tree(repo_root, plan)
    }

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
        plan.android
            .as_ref()
            .map(|android| android.generated_root.clone())
    }
}

fn build_android_plan(
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

fn render_android_build_file(
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
    render_template(
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

fn render_android_manifest_xml(android: &AndroidConfig, manifest: &JsonMap) -> AtomResult<String> {
    render_android_manifest_document(
        android.application_id.as_deref().unwrap_or_default(),
        manifest,
    )
}

fn render_kotlin_application(app: &AppConfig, android: &AndroidConfig) -> AtomResult<String> {
    render_template(
        "android/AtomApplication.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            name => &app.name,
            slug => &app.slug,
        },
    )
}

fn render_kotlin_bindings(
    modules: &[ResolvedModule],
    android: &AndroidConfig,
) -> AtomResult<String> {
    let module_ids: Vec<&str> = modules.iter().map(|m| m.manifest.id.as_str()).collect();
    render_template(
        "android/AtomBindings.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            module_ids,
        },
    )
}

fn render_kotlin_main_activity(app: &AppConfig, android: &AndroidConfig) -> AtomResult<String> {
    render_template(
        "android/MainActivity.kt",
        context! {
            package_name => android.application_id.as_deref().unwrap_or_default(),
            name => &app.name,
            slug => &app.slug,
        },
    )
}

fn render_android_runtime_jni(app: &AppConfig, android: &AndroidConfig) -> AtomResult<String> {
    render_template(
        "android/atom_runtime_jni.rs",
        context! {
            entry_crate_name => &app.entry_crate_name,
            jni_prefix => jni_prefix(android.application_id.as_deref().unwrap_or_default()),
        },
    )
}

fn kotlin_package_dir(application_id: &str) -> Utf8PathBuf {
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

fn emit_android_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    let Some(android) = &plan.android else {
        return Ok(());
    };
    let android_config = plan
        .android_config
        .as_ref()
        .expect("android config should exist when android output exists");
    write_generated_file(
        &repo_root.join(&android.generated_root).join("BUILD.bazel"),
        &render_android_build_file(
            &plan.app,
            &plan.modules,
            android_config,
            &plan.android_resources,
        )?,
    )?;
    write_generated_file(
        &repo_root
            .join(&android.generated_root)
            .join("AndroidManifest.generated.xml"),
        &render_android_manifest_xml(android_config, &plan.android_manifest)?,
    )?;
    write_generated_file(
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
    write_generated_file(
        &repo_root.join(&package_dir).join("AtomApplication.kt"),
        &render_kotlin_application(&plan.app, android_config)?,
    )?;
    write_generated_file(
        &repo_root.join(&package_dir).join("AtomBindings.kt"),
        &render_kotlin_bindings(&plan.modules, android_config)?,
    )?;
    write_generated_file(
        &repo_root.join(&package_dir).join("MainActivity.kt"),
        &render_kotlin_main_activity(&plan.app, android_config)?,
    )?;
    Ok(())
}
