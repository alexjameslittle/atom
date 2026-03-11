use atom_backends::{
    BackendDefinition, GenerationBackend, GenerationBackendRegistry, GenerationPlan, PlatformPlan,
};
use atom_cng::{render_plist_document, render_template, static_template, write_generated_file};
use atom_ffi::AtomResult;
use atom_manifest::{AppConfig, BuildConfig, IosConfig, NormalizedManifest, metadata_target};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use minijinja::context;

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
    fn build_platform_plan(&self, manifest: &NormalizedManifest) -> Option<PlatformPlan> {
        manifest
            .ios
            .enabled
            .then(|| build_ios_plan(&manifest.app, &manifest.build, &manifest.ios))
    }

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
        emit_ios_host_tree(repo_root, plan)
    }

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
        plan.ios.as_ref().map(|ios| ios.generated_root.clone())
    }
}

fn build_ios_plan(app: &AppConfig, build: &BuildConfig, _ios: &IosConfig) -> PlatformPlan {
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
    PlatformPlan {
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

fn emit_ios_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
    let Some(ios) = &plan.ios else {
        return Ok(());
    };
    let ios_config = plan
        .ios_config
        .as_ref()
        .expect("ios config should exist when ios output exists");
    write_generated_file(
        &repo_root.join(&ios.generated_root).join("BUILD.bazel"),
        &render_ios_build_file(
            &plan.app,
            &plan.modules,
            ios_config,
            &plan.ios_resources,
            &plan.ios_resource_globs,
        )?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("Info.generated.plist"),
        &render_ios_plist(&plan.plist)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("LaunchScreen.storyboard"),
        &render_ios_launch_storyboard()?,
    )?;
    write_generated_file(
        &repo_root.join(&ios.generated_root).join("atom_runtime.h"),
        &render_ios_runtime_header()?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("atom_runtime_app_bridge.rs"),
        &render_ios_runtime_bridge(&plan.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("AtomAppDelegate.swift"),
        &render_swift_app_delegate(&plan.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("SceneDelegate.swift"),
        &render_swift_scene_delegate(&plan.app)?,
    )?;
    write_generated_file(
        &repo_root
            .join(&ios.generated_root)
            .join("AtomBindings.swift"),
        &render_swift_bindings(&plan.modules)?,
    )?;
    write_generated_file(
        &repo_root.join(&ios.generated_root).join("main.swift"),
        &render_swift_main()?,
    )?;
    Ok(())
}
