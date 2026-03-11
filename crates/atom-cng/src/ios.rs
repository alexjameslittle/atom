use atom_ffi::AtomResult;
use atom_manifest::{AppConfig, BuildConfig, IosConfig, metadata_target};
use atom_modules::{JsonMap, ResolvedModule};
use minijinja::context;

use crate::PlatformPlan;
use crate::templates::render;

pub(crate) fn build_ios_plan(
    app: &AppConfig,
    build: &BuildConfig,
    _ios: &IosConfig,
) -> PlatformPlan {
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

pub(crate) fn render_ios_build_file(
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
    render(
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

pub(crate) fn render_ios_plist(plist: &JsonMap) -> AtomResult<String> {
    crate::render_plist_document(plist)
}

pub(crate) fn render_ios_launch_storyboard() -> String {
    include_str!("templates/ios/LaunchScreen.storyboard").to_owned()
}

pub(crate) fn render_ios_runtime_header() -> String {
    include_str!("templates/ios/atom_runtime.h").to_owned()
}

pub(crate) fn render_ios_runtime_bridge(app: &AppConfig) -> AtomResult<String> {
    render(
        "ios/atom_runtime_app_bridge.rs",
        context! {
            entry_crate_name => &app.entry_crate_name,
        },
    )
}

pub(crate) fn render_swift_app_delegate(_app: &AppConfig) -> String {
    include_str!("templates/ios/AtomAppDelegate.swift").to_owned()
}

pub(crate) fn render_swift_scene_delegate(app: &AppConfig) -> AtomResult<String> {
    render(
        "ios/SceneDelegate.swift",
        context! {
            name => &app.name,
            slug => &app.slug,
            support_module => swift_support_module_name(app),
        },
    )
}

pub(crate) fn render_swift_main() -> String {
    include_str!("templates/ios/main.swift").to_owned()
}

pub(crate) fn render_swift_bindings(modules: &[ResolvedModule]) -> AtomResult<String> {
    let module_ids: Vec<&str> = modules.iter().map(|m| m.manifest.id.as_str()).collect();
    render(
        "ios/AtomBindings.swift",
        context! {
            module_ids,
        },
    )
}

pub(crate) fn swift_support_module_name(app: &AppConfig) -> String {
    format!("atom_{}_support", app.slug.replace('-', "_"))
}
