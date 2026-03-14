use atom_backends::{
    BackendContribution, BackendDefinition, BackendPlan, GenerationBackend,
    GenerationBackendRegistry, GenerationPlan,
};
use atom_cng::{render_rust_module_exports, write_generated_file};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    AndroidConfig, AppConfig, BuildConfig, ConfigPluginRequest, NormalizedManifest, metadata_target,
};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;
use serde_json::{Value, json};

use crate::templates::render as render_template;

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
    fn initialize_backend(
        &self,
        manifest: &NormalizedManifest,
    ) -> AtomResult<Option<BackendContribution>> {
        Ok(manifest.android.enabled.then(|| BackendContribution {
            metadata_entries: default_android_manifest(&manifest.app, &manifest.android),
            ..BackendContribution::default()
        }))
    }

    fn module_contribution(
        &self,
        _manifest: &NormalizedManifest,
        module: &ResolvedModule,
    ) -> AtomResult<BackendContribution> {
        Ok(BackendContribution {
            metadata_entries: module.manifest.android_manifest.clone(),
            ..BackendContribution::default()
        })
    }

    fn validate_module_compatibility(
        &self,
        manifest: &NormalizedManifest,
        module: &ResolvedModule,
    ) -> AtomResult<()> {
        validate_min_sdk(
            manifest.android.min_sdk,
            module.manifest.android_min_sdk,
            module.manifest.target_label.as_str(),
            "android_min_sdk",
        )
    }

    fn validate_config_plugin_compatibility(
        &self,
        manifest: &NormalizedManifest,
        plugin: &ConfigPluginRequest,
    ) -> AtomResult<()> {
        validate_min_sdk(
            manifest.android.min_sdk,
            plugin.android_min_sdk,
            plugin.target_label.as_str(),
            "android_min_sdk",
        )
    }

    fn build_backend_plan(&self, manifest: &NormalizedManifest) -> Option<BackendPlan> {
        manifest
            .android
            .enabled
            .then(|| build_android_plan(&manifest.app, &manifest.build, &manifest.android))
    }

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
        emit_android_host_tree(repo_root, plan)
    }

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
        plan.backend(BACKEND_ID)
            .map(|android| android.plan.generated_root.clone())
    }
}

fn build_android_plan(
    app: &AppConfig,
    build: &BuildConfig,
    android: &AndroidConfig,
) -> BackendPlan {
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
    BackendPlan {
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
    let rust_module_labels: Vec<&str> = modules
        .iter()
        .filter(|module| module.manifest.kind == atom_modules::ModuleKind::Rust)
        .map(|module| module.request.target_label.as_str())
        .collect();
    render_template(
        "android/BUILD.bazel",
        context! {
            jni_prefix => jni_prefix(package_name),
            entry_crate_label => &app.entry_crate_label,
            source_root => source_root.as_str(),
            module_labels,
            rust_module_labels,
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

fn render_android_runtime_jni(
    repo_root: &Utf8Path,
    app: &AppConfig,
    android: &AndroidConfig,
    modules: &[ResolvedModule],
) -> AtomResult<String> {
    let module_exports = render_rust_module_exports(repo_root, modules)?;
    render_template(
        "android/atom_runtime_jni.rs",
        context! {
            entry_crate_name => &app.entry_crate_name,
            jni_prefix => jni_prefix(android.application_id.as_deref().unwrap_or_default()),
            module_exports,
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

fn default_android_manifest(app: &AppConfig, android: &AndroidConfig) -> JsonMap {
    object_from_value(json!({
        "uses-sdk": {
            "@android:minSdkVersion": android.min_sdk.unwrap_or_default(),
            "@android:targetSdkVersion": android.target_sdk.unwrap_or_default(),
        },
        "application": {
            "@android:label": app.name,
            "@android:name": ".AtomApplication",
            "activity": {
                "@android:name": ".MainActivity",
                "@android:exported": true,
                "intent-filter": {
                    "action": {
                        "@android:name": "android.intent.action.MAIN"
                    },
                    "category": {
                        "@android:name": "android.intent.category.LAUNCHER"
                    }
                }
            }
        }
    }))
}

fn validate_min_sdk(
    current: Option<u32>,
    required: Option<u32>,
    target_label: &str,
    field: &str,
) -> AtomResult<()> {
    if let (Some(current), Some(required)) = (current, required)
        && current < required
    {
        return Err(extension_compatibility_error(
            target_label,
            field,
            format!(
                "extension requires Android min_sdk {required}, app is configured for {current}"
            ),
        ));
    }
    Ok(())
}

fn extension_compatibility_error(target_label: &str, field: &str, message: String) -> AtomError {
    AtomError::with_path(
        AtomErrorCode::ExtensionIncompatible,
        message,
        format!("{target_label}.{field}"),
    )
}

fn object_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map,
        _ => JsonMap::new(),
    }
}

fn render_android_manifest_document(package_name: &str, manifest: &JsonMap) -> AtomResult<String> {
    use std::fmt::Write;

    let mut output = String::new();
    writeln!(
        output,
        "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\" package=\"{}\">",
        xml_escape(package_name)
    )
    .expect("write to string");
    render_android_nodes(&mut output, manifest, 1)?;
    output.push_str("</manifest>\n");
    Ok(output)
}

fn render_android_nodes(output: &mut String, nodes: &JsonMap, indent: usize) -> AtomResult<()> {
    for (name, value) in nodes {
        render_android_node(output, name, value, indent)?;
    }
    Ok(())
}

fn render_android_node(
    output: &mut String,
    name: &str,
    value: &Value,
    indent: usize,
) -> AtomResult<()> {
    use std::fmt::Write;

    if let Value::Array(values) = value {
        for entry in values {
            render_android_node(output, name, entry, indent)?;
        }
        return Ok(());
    }

    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            let mut attributes = Vec::new();
            let mut children = JsonMap::new();
            let mut text = None;
            for (key, entry) in map {
                if let Some(attribute) = key.strip_prefix('@') {
                    attributes.push((attribute, entry));
                } else if key == "#text" {
                    text = Some(entry);
                } else {
                    children.insert(key.clone(), entry.clone());
                }
            }

            write!(output, "{prefix}<{name}").expect("write");
            for (attribute, entry) in attributes {
                write!(
                    output,
                    " {attribute}=\"{}\"",
                    xml_escape(&render_xml_scalar(entry)?)
                )
                .expect("write");
            }

            if children.is_empty() && text.is_none() {
                output.push_str(" />\n");
                return Ok(());
            }

            output.push('>');
            if let Some(text) = text {
                write!(output, "{}", xml_escape(&render_xml_scalar(text)?)).expect("write");
            }
            if !children.is_empty() {
                output.push('\n');
                render_android_nodes(output, &children, indent + 1)?;
                write!(output, "{prefix}").expect("write");
            }
            writeln!(output, "</{name}>").expect("write");
        }
        Value::String(_) | Value::Bool(_) | Value::Number(_) => {
            writeln!(
                output,
                "{prefix}<{name}>{}</{name}>",
                xml_escape(&render_xml_scalar(value)?)
            )
            .expect("write");
        }
        Value::Null => {
            return Err(AtomError::new(
                AtomErrorCode::CngTemplateError,
                "android manifest values must not be null",
            ));
        }
        Value::Array(_) => unreachable!("arrays are handled above"),
    }
    Ok(())
}

fn render_xml_scalar(value: &Value) -> AtomResult<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "android manifest attribute values must be scalar",
        )),
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn emit_android_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    let Some(android) = plan.backend(BACKEND_ID) else {
        return Ok(());
    };
    let android_config = &plan.manifest.android;
    write_generated_file(
        &repo_root
            .join(&android.plan.generated_root)
            .join("BUILD.bazel"),
        &render_android_build_file(
            &plan.manifest.app,
            &plan.modules,
            android_config,
            &android.bazel_resources,
        )?,
    )?;
    write_generated_file(
        &repo_root
            .join(&android.plan.generated_root)
            .join("AndroidManifest.generated.xml"),
        &render_android_manifest_xml(android_config, &android.metadata)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&android.plan.generated_root)
            .join("atom_runtime_jni.rs"),
        &render_android_runtime_jni(repo_root, &plan.manifest.app, android_config, &plan.modules)?,
    )?;
    let package_dir = android
        .plan
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
        &render_kotlin_application(&plan.manifest.app, android_config)?,
    )?;
    write_generated_file(
        &repo_root.join(&package_dir).join("AtomBindings.kt"),
        &render_kotlin_bindings(&plan.modules, android_config)?,
    )?;
    write_generated_file(
        &repo_root.join(&package_dir).join("MainActivity.kt"),
        &render_kotlin_main_activity(&plan.manifest.app, android_config)?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_backends::{BackendDefinition, GenerationBackendRegistry};
    use atom_cng::{ConfigPluginRegistry, build_generation_plan, emit_host_tree};
    use atom_manifest::testing::fixture_manifest;
    use atom_modules::testing::fixture_resolved_module;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{BACKEND_ID, register};

    #[test]
    fn registers_generation_backend() {
        let mut registry = GenerationBackendRegistry::new();
        register(&mut registry).expect("android backend should register");

        assert_eq!(
            registry
                .get(BACKEND_ID)
                .expect("android backend should be available")
                .id(),
            BACKEND_ID
        );
    }

    #[test]
    fn emits_expected_android_host_tree() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = fixture_manifest(&root);
        let modules = Vec::new();
        let mut registry = GenerationBackendRegistry::new();
        register(&mut registry).expect("android backend should register");

        let plan = build_generation_plan(
            &manifest,
            &modules,
            &ConfigPluginRegistry::default(),
            &registry,
        )
        .expect("plan");
        emit_host_tree(&root, &plan, &registry).expect("host tree");

        let build_file =
            fs::read_to_string(root.join("generated/android/fixture/BUILD.bazel")).expect("build");
        let manifest_xml = fs::read_to_string(
            root.join("generated/android/fixture/AndroidManifest.generated.xml"),
        )
        .expect("manifest");
        let bridge = fs::read_to_string(root.join("generated/android/fixture/atom_runtime_jni.rs"))
            .expect("bridge");
        let app = fs::read_to_string(root.join(
            "generated/android/fixture/src/main/kotlin/build/atom/fixture/AtomApplication.kt",
        ))
        .expect("application");
        let bindings =
            fs::read_to_string(root.join(
                "generated/android/fixture/src/main/kotlin/build/atom/fixture/AtomBindings.kt",
            ))
            .expect("bindings");

        assert!(build_file.contains("android_binary("));
        assert!(build_file.contains("custom_package = \"build.atom.fixture\""));
        assert!(build_file.contains("\"@atom//crates/atom-runtime\""));
        assert!(build_file.contains("\"@atom//crates/atom-ffi\""));
        assert!(manifest_xml.contains("android:minSdkVersion=\"28\""));
        assert!(manifest_xml.contains("android:targetSdkVersion=\"35\""));
        assert!(bridge.contains("fixture::atom_runtime_config()"));
        assert!(app.contains("class AtomApplication : Application()"));
        assert!(bindings.contains("val modules: List<String> = listOf("));
    }

    #[test]
    fn rejects_modules_that_require_higher_min_sdk() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = fixture_manifest(&root);
        let mut module = fixture_resolved_module(&root);
        module.manifest.android_min_sdk = Some(36);
        let modules = vec![module];
        let mut registry = GenerationBackendRegistry::new();
        register(&mut registry).expect("android backend should register");

        let error = build_generation_plan(
            &manifest,
            &modules,
            &ConfigPluginRegistry::default(),
            &registry,
        )
        .expect_err("higher min sdk should fail");

        assert_eq!(error.code, atom_ffi::AtomErrorCode::ExtensionIncompatible);
        assert!(error.message.contains("min_sdk"));
    }
}
