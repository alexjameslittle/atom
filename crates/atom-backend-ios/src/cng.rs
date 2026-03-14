use atom_backends::{
    BackendContribution, BackendDefinition, BackendPlan, GenerationBackend,
    GenerationBackendRegistry, GenerationPlan,
};
use atom_cng::write_generated_file;
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    AppConfig, BuildConfig, ConfigPluginRequest, IosConfig, NormalizedManifest, metadata_target,
};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;
use serde_json::{Value, json};

use crate::templates::{render as render_template, static_template};

const BACKEND_ID: &str = "ios";

struct IosGenerationBackend;

pub fn register(registry: &mut GenerationBackendRegistry) -> AtomResult<()> {
    registry.register(Box::new(IosGenerationBackend))
}

impl BackendDefinition for IosGenerationBackend {
    fn id(&self) -> &'static str {
        BACKEND_ID
    }

    fn platform(&self) -> &'static str {
        "ios"
    }
}

impl GenerationBackend for IosGenerationBackend {
    fn initialize_backend(
        &self,
        manifest: &NormalizedManifest,
    ) -> AtomResult<Option<BackendContribution>> {
        Ok(manifest.ios.enabled.then(|| BackendContribution {
            metadata_entries: default_ios_plist(&manifest.app, &manifest.ios),
            ..BackendContribution::default()
        }))
    }

    fn module_contribution(
        &self,
        _manifest: &NormalizedManifest,
        module: &ResolvedModule,
    ) -> AtomResult<BackendContribution> {
        Ok(BackendContribution {
            metadata_entries: module.manifest.plist.clone(),
            ..BackendContribution::default()
        })
    }

    fn validate_module_compatibility(
        &self,
        manifest: &NormalizedManifest,
        module: &ResolvedModule,
    ) -> AtomResult<()> {
        validate_deployment_target(
            manifest.ios.deployment_target.as_deref(),
            module.manifest.ios_min_deployment_target.as_deref(),
            module.manifest.target_label.as_str(),
            "ios_min_deployment_target",
        )
    }

    fn validate_config_plugin_compatibility(
        &self,
        manifest: &NormalizedManifest,
        plugin: &ConfigPluginRequest,
    ) -> AtomResult<()> {
        validate_deployment_target(
            manifest.ios.deployment_target.as_deref(),
            plugin.ios_min_deployment_target.as_deref(),
            plugin.target_label.as_str(),
            "ios_min_deployment_target",
        )
    }

    fn build_backend_plan(&self, manifest: &NormalizedManifest) -> Option<BackendPlan> {
        manifest
            .ios
            .enabled
            .then(|| build_ios_plan(&manifest.app, &manifest.build, &manifest.ios))
    }

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
        emit_ios_host_tree(repo_root, plan)
    }

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
        plan.backend(BACKEND_ID)
            .map(|ios| ios.plan.generated_root.clone())
    }
}

fn build_ios_plan(app: &AppConfig, build: &BuildConfig, _ios: &IosConfig) -> BackendPlan {
    let generated_root = build.generated_root.join("ios").join(&app.slug);
    let files = vec![
        generated_root.join("BUILD.bazel"),
        generated_root.join("Info.generated.plist"),
        generated_root.join("LaunchScreen.storyboard"),
        generated_root.join("atom_runtime.h"),
        generated_root.join("atom_runtime_app_bridge.rs"),
        generated_root.join("AtomAppDelegate.swift"),
        generated_root.join("SceneDelegate.swift"),
        generated_root.join("AtomBindings.swift"),
        generated_root.join("main.swift"),
    ];
    BackendPlan {
        target: format!("//{}:app", generated_root.as_str()),
        generated_root,
        files,
    }
}

fn render_ios_build_file(
    app: &AppConfig,
    modules: &[ResolvedModule],
    ios: &IosConfig,
    extra_resources: &[String],
    extra_resource_globs: &[String],
) -> AtomResult<String> {
    let module_labels: Vec<String> = modules
        .iter()
        .map(|m| metadata_target(&m.request.target_label, "_ios_srcs"))
        .collect::<AtomResult<_>>()?;
    render_template(
        "ios/BUILD.bazel",
        context! {
            support_module => swift_support_module_name(app),
            entry_crate_label => &app.entry_crate_label,
            module_labels,
            bundle_id => ios.bundle_id.as_deref().unwrap_or_default(),
            deployment_target => ios.deployment_target.as_deref().unwrap_or_default(),
            extra_resources,
            extra_resource_globs,
        },
    )
}

fn render_ios_plist(plist: &JsonMap) -> AtomResult<String> {
    render_plist_document(plist)
}

fn render_ios_launch_storyboard() -> AtomResult<String> {
    Ok(static_template("ios/LaunchScreen.storyboard")?.to_owned())
}

fn render_ios_runtime_header() -> AtomResult<String> {
    Ok(static_template("ios/atom_runtime.h")?.to_owned())
}

fn render_ios_runtime_bridge(app: &AppConfig) -> AtomResult<String> {
    render_template(
        "ios/atom_runtime_app_bridge.rs",
        context! {
            entry_crate_name => &app.entry_crate_name,
        },
    )
}

fn render_swift_app_delegate(_app: &AppConfig) -> AtomResult<String> {
    Ok(static_template("ios/AtomAppDelegate.swift")?.to_owned())
}

fn render_swift_scene_delegate(app: &AppConfig) -> AtomResult<String> {
    render_template(
        "ios/SceneDelegate.swift",
        context! {
            name => &app.name,
            slug => &app.slug,
            support_module => swift_support_module_name(app),
        },
    )
}

fn render_swift_main() -> AtomResult<String> {
    Ok(static_template("ios/main.swift")?.to_owned())
}

fn render_swift_bindings(modules: &[ResolvedModule]) -> AtomResult<String> {
    let module_ids: Vec<&str> = modules.iter().map(|m| m.manifest.id.as_str()).collect();
    render_template(
        "ios/AtomBindings.swift",
        context! {
            module_ids,
        },
    )
}

fn swift_support_module_name(app: &AppConfig) -> String {
    format!("atom_{}_support", app.slug.replace('-', "_"))
}

fn default_ios_plist(app: &AppConfig, ios: &IosConfig) -> JsonMap {
    object_from_value(json!({
        "CFBundleName": app.slug,
        "CFBundleDisplayName": app.name,
        "CFBundleIdentifier": ios.bundle_id.as_deref().unwrap_or_default(),
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "1.0",
        "CFBundleVersion": "1",
        "LSRequiresIPhoneOS": true,
        "MinimumOSVersion": ios.deployment_target.as_deref().unwrap_or_default(),
        "UIApplicationSupportsIndirectInputEvents": true,
        "UIApplicationSceneManifest": {
            "UIApplicationSupportsMultipleScenes": false,
            "UISceneConfigurations": {
                "UIWindowSceneSessionRoleApplication": [
                    {
                        "UISceneConfigurationName": "Default Configuration",
                        "UISceneDelegateClassName": format!(
                            "{}.AtomSceneDelegate",
                            swift_support_module_name(app)
                        ),
                    }
                ]
            }
        },
        "UIMainStoryboardFile": "",
        "UILaunchStoryboardName": "LaunchScreen.storyboard",
        "UISupportedInterfaceOrientations": [
            "UIInterfaceOrientationPortrait"
        ]
    }))
}

fn validate_deployment_target(
    current: Option<&str>,
    required: Option<&str>,
    target_label: &str,
    field: &str,
) -> AtomResult<()> {
    if let (Some(current), Some(required)) = (current, required)
        && compare_deployment_target(current, required)? == std::cmp::Ordering::Less
    {
        return Err(extension_compatibility_error(
            target_label,
            field,
            format!(
                "extension requires iOS deployment target {required}, app is configured for {current}"
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

fn compare_deployment_target(current: &str, required: &str) -> AtomResult<std::cmp::Ordering> {
    let current = parse_deployment_target(current)?;
    let required = parse_deployment_target(required)?;
    Ok(current.cmp(&required))
}

fn parse_deployment_target(value: &str) -> AtomResult<(u32, u32)> {
    let mut components = value.split('.');
    match (components.next(), components.next(), components.next()) {
        (Some(major), Some(minor), None) => Ok((
            parse_u32_component(major, "deployment target")?,
            parse_u32_component(minor, "deployment target")?,
        )),
        _ => Err(AtomError::new(
            AtomErrorCode::InternalBug,
            format!("invalid deployment target format: {value}"),
        )),
    }
}

fn parse_u32_component(value: &str, kind: &str) -> AtomResult<u32> {
    value.parse::<u32>().map_err(|error| {
        AtomError::new(
            AtomErrorCode::InternalBug,
            format!("failed to parse {kind} component {value}: {error}"),
        )
    })
}

fn object_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map,
        _ => JsonMap::new(),
    }
}

fn render_plist_document(plist: &JsonMap) -> AtomResult<String> {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n",
    );
    render_plist_value(&mut output, &Value::Object(plist.clone()), 0)?;
    output.push_str("</plist>\n");
    Ok(output)
}

fn render_plist_value(output: &mut String, value: &Value, indent: usize) -> AtomResult<()> {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            use std::fmt::Write;

            writeln!(output, "{prefix}<dict>").expect("write to string");
            for (key, entry) in map {
                writeln!(output, "{prefix}  <key>{}</key>", xml_escape(key)).expect("write");
                render_plist_value(output, entry, indent + 1)?;
            }
            writeln!(output, "{prefix}</dict>").expect("write to string");
        }
        Value::Array(values) => {
            use std::fmt::Write;

            writeln!(output, "{prefix}<array>").expect("write to string");
            for entry in values {
                render_plist_value(output, entry, indent + 1)?;
            }
            writeln!(output, "{prefix}</array>").expect("write to string");
        }
        Value::String(value) => {
            use std::fmt::Write;

            writeln!(output, "{prefix}<string>{}</string>", xml_escape(value)).expect("write");
        }
        Value::Bool(true) => {
            use std::fmt::Write;

            writeln!(output, "{prefix}<true/>").expect("write to string");
        }
        Value::Bool(false) => {
            use std::fmt::Write;

            writeln!(output, "{prefix}<false/>").expect("write to string");
        }
        Value::Number(number) => {
            use std::fmt::Write;

            if number.is_i64() || number.is_u64() {
                writeln!(output, "{prefix}<integer>{number}</integer>").expect("write");
            } else {
                writeln!(output, "{prefix}<real>{number}</real>").expect("write");
            }
        }
        Value::Null => {
            return Err(AtomError::new(
                AtomErrorCode::CngTemplateError,
                "plist values must not be null",
            ));
        }
    }
    Ok(())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn emit_ios_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    let Some(ios) = plan.backend(BACKEND_ID) else {
        return Ok(());
    };
    let ios_config = &plan.manifest.ios;
    write_generated_file(
        &repo_root.join(&ios.plan.generated_root).join("BUILD.bazel"),
        &render_ios_build_file(
            &plan.manifest.app,
            &plan.modules,
            ios_config,
            &ios.bazel_resources,
            &ios.bazel_resource_globs,
        )?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("Info.generated.plist"),
        &render_ios_plist(&ios.metadata)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("LaunchScreen.storyboard"),
        &render_ios_launch_storyboard()?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("atom_runtime.h"),
        &render_ios_runtime_header()?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("atom_runtime_app_bridge.rs"),
        &render_ios_runtime_bridge(&plan.manifest.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("AtomAppDelegate.swift"),
        &render_swift_app_delegate(&plan.manifest.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("SceneDelegate.swift"),
        &render_swift_scene_delegate(&plan.manifest.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.plan.generated_root)
            .join("AtomBindings.swift"),
        &render_swift_bindings(&plan.modules)?,
    )?;
    write_generated_file(
        &repo_root.join(&ios.plan.generated_root).join("main.swift"),
        &render_swift_main()?,
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
        register(&mut registry).expect("ios backend should register");

        assert_eq!(
            registry
                .get(BACKEND_ID)
                .expect("ios backend should be available")
                .id(),
            BACKEND_ID
        );
    }

    #[test]
    fn emits_expected_ios_host_tree() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = fixture_manifest(&root);
        let modules = Vec::new();
        let mut registry = GenerationBackendRegistry::new();
        register(&mut registry).expect("ios backend should register");

        let plan = build_generation_plan(
            &manifest,
            &modules,
            &ConfigPluginRegistry::default(),
            &registry,
        )
        .expect("plan");
        emit_host_tree(&root, &plan, &registry).expect("host tree");

        let build_file =
            fs::read_to_string(root.join("generated/ios/fixture/BUILD.bazel")).expect("build");
        let plist = fs::read_to_string(root.join("generated/ios/fixture/Info.generated.plist"))
            .expect("plist");
        let bridge =
            fs::read_to_string(root.join("generated/ios/fixture/atom_runtime_app_bridge.rs"))
                .expect("bridge");
        let scene = fs::read_to_string(root.join("generated/ios/fixture/SceneDelegate.swift"))
            .expect("scene");
        let bindings = fs::read_to_string(root.join("generated/ios/fixture/AtomBindings.swift"))
            .expect("bindings");

        assert!(build_file.contains("ios_application("));
        assert!(build_file.contains("bundle_id = \"build.atom.fixture\""));
        assert!(build_file.contains("minimum_os_version = \"17.0\""));
        assert!(build_file.contains("\"@atom//crates/atom-runtime\""));
        assert!(build_file.contains("\"@atom//crates/atom-ffi\""));
        assert!(plist.contains("<key>CFBundleIdentifier</key>"));
        assert!(plist.contains("<string>build.atom.fixture</string>"));
        assert!(bridge.contains("fixture::atom_runtime_config()"));
        assert!(bindings.contains("public static let modules: [String] = ["));
        assert!(scene.contains("AtomHostRootViewProvider"));
    }

    #[test]
    fn rejects_modules_that_require_higher_deployment_targets() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let manifest = fixture_manifest(&root);
        let mut module = fixture_resolved_module(&root);
        module.manifest.ios_min_deployment_target = Some("18.0".to_owned());
        let modules = vec![module];
        let mut registry = GenerationBackendRegistry::new();
        register(&mut registry).expect("ios backend should register");

        let error = build_generation_plan(
            &manifest,
            &modules,
            &ConfigPluginRegistry::default(),
            &registry,
        )
        .expect_err("higher deployment target should fail");

        assert_eq!(error.code, atom_ffi::AtomErrorCode::ExtensionIncompatible);
        assert!(error.message.contains("deployment target"));
    }
}
