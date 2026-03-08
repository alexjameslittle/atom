use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest, metadata_target,
};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformPlan {
    pub generated_root: Utf8PathBuf,
    pub target: String,
    pub files: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaPlan {
    pub aggregate: Utf8PathBuf,
    pub modules: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaFilePlan {
    pub source: Utf8PathBuf,
    pub output: Utf8PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerationPlan {
    pub version: u16,
    pub status: String,
    pub app: AppConfig,
    pub build: BuildConfig,
    pub ios_config: Option<IosConfig>,
    pub android_config: Option<AndroidConfig>,
    pub modules: Vec<ResolvedModule>,
    pub permissions: Vec<String>,
    pub plist: JsonMap,
    pub android_manifest: JsonMap,
    pub entitlements: JsonMap,
    pub schema: SchemaPlan,
    pub schema_files: Vec<SchemaFilePlan>,
    pub ios: Option<PlatformPlan>,
    pub android: Option<PlatformPlan>,
    pub generated_files: Vec<Utf8PathBuf>,
    pub warnings: Vec<String>,
}

/// # Errors
///
/// Returns an error if module metadata merging produces a conflict.
pub fn build_generation_plan(
    manifest: &NormalizedManifest,
    modules: &[ResolvedModule],
) -> AtomResult<GenerationPlan> {
    let mut permissions = BTreeSet::new();
    let mut plist = JsonMap::new();
    let mut android_manifest = JsonMap::new();
    let mut entitlements = JsonMap::new();
    let mut schema_outputs = Vec::new();
    let schema_root = manifest.build.generated_root.join("schema");
    let aggregate_schema = schema_root.join("atom.fbs");

    for module in modules {
        for permission in &module.manifest.permissions {
            permissions.insert(permission.clone());
        }
        deep_merge_map(
            &mut plist,
            &module.manifest.plist,
            &format!("plist.{}", module.manifest.id),
        )?;
        deep_merge_map(
            &mut android_manifest,
            &module.manifest.android_manifest,
            &format!("android_manifest.{}", module.manifest.id),
        )?;
        deep_merge_map(
            &mut entitlements,
            &module.manifest.entitlements,
            &format!("entitlements.{}", module.manifest.id),
        )?;

        for schema_file in &module.manifest.schema_files {
            let relative = normalize_schema_output(schema_file);
            schema_outputs.push(SchemaFilePlan {
                source: manifest.repo_root.join(schema_file),
                output: schema_root
                    .join("modules")
                    .join(&module.manifest.id)
                    .join(relative),
            });
        }
    }

    let schema = SchemaPlan {
        aggregate: aggregate_schema.clone(),
        modules: schema_outputs
            .iter()
            .map(|schema_file| schema_file.output.clone())
            .collect(),
    };

    let ios = manifest
        .ios
        .enabled
        .then(|| build_ios_plan(&manifest.app, &manifest.build, &manifest.ios));
    let android = manifest
        .android
        .enabled
        .then(|| build_android_plan(&manifest.app, &manifest.build, &manifest.android));

    let mut generated_files = vec![aggregate_schema];
    generated_files.extend(schema.modules.iter().cloned());
    if let Some(ios) = &ios {
        generated_files.extend(ios.files.iter().cloned());
    }
    if let Some(android) = &android {
        generated_files.extend(android.files.iter().cloned());
    }

    Ok(GenerationPlan {
        version: 1,
        status: "dry-run".to_owned(),
        app: manifest.app.clone(),
        build: manifest.build.clone(),
        ios_config: manifest.ios.enabled.then_some(manifest.ios.clone()),
        android_config: manifest.android.enabled.then_some(manifest.android.clone()),
        modules: modules.to_vec(),
        permissions: permissions.into_iter().collect(),
        plist,
        android_manifest,
        entitlements,
        schema,
        schema_files: schema_outputs,
        ios,
        android,
        generated_files,
        warnings: Vec::new(),
    })
}

#[must_use]
pub fn render_prebuild_plan(plan: &GenerationPlan) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();

    let status = builder.create_string(&plan.status);
    let app_name = builder.create_string(&plan.app.name);
    let app_slug = builder.create_string(&plan.app.slug);
    let app_entry_target = builder.create_string(&plan.app.entry_crate_label);
    let app = create_prebuild_app(&mut builder, app_name, app_slug, app_entry_target);

    let mut module_offsets = Vec::with_capacity(plan.modules.len());
    for module in &plan.modules {
        let id = builder.create_string(&module.manifest.id);
        let target_label = builder.create_string(&module.request.target_label);
        module_offsets.push(create_prebuild_module(
            &mut builder,
            id,
            u32::try_from(module.init_order).unwrap_or(u32::MAX),
            target_label,
        ));
    }
    let modules = builder.create_vector(module_offsets.as_slice());

    let ios = plan.ios.as_ref().map(|ios| {
        let generated_root = builder.create_string(ios.generated_root.as_str());
        let target = builder.create_string(&ios.target);
        create_prebuild_platform(&mut builder, generated_root, target)
    });
    let android = plan.android.as_ref().map(|android| {
        let generated_root = builder.create_string(android.generated_root.as_str());
        let target = builder.create_string(&android.target);
        create_prebuild_platform(&mut builder, generated_root, target)
    });

    let aggregate = builder.create_string(plan.schema.aggregate.as_str());
    let mut schema_module_offsets = Vec::with_capacity(plan.schema.modules.len());
    for module_schema in &plan.schema.modules {
        schema_module_offsets.push(builder.create_string(module_schema.as_str()));
    }
    let schema_modules = builder.create_vector(schema_module_offsets.as_slice());
    let schema = create_prebuild_schema(&mut builder, aggregate, schema_modules);

    let mut generated_file_offsets = Vec::with_capacity(plan.generated_files.len());
    for file in &plan.generated_files {
        generated_file_offsets.push(builder.create_string(file.as_str()));
    }
    let generated_files = builder.create_vector(generated_file_offsets.as_slice());

    let mut warning_offsets = Vec::with_capacity(plan.warnings.len());
    for warning in &plan.warnings {
        warning_offsets.push(builder.create_string(warning));
    }
    let warnings = builder.create_vector(warning_offsets.as_slice());

    let root = {
        let table = builder.start_table();
        builder.push_slot::<u16>(4, plan.version, 0);
        builder.push_slot_always::<WIPOffset<_>>(6, status);
        builder.push_slot_always::<WIPOffset<_>>(8, app);
        builder.push_slot_always::<WIPOffset<_>>(10, modules);
        if let Some(ios) = ios {
            builder.push_slot_always::<WIPOffset<_>>(12, ios);
        }
        if let Some(android) = android {
            builder.push_slot_always::<WIPOffset<_>>(14, android);
        }
        builder.push_slot_always::<WIPOffset<_>>(16, schema);
        builder.push_slot_always::<WIPOffset<_>>(18, generated_files);
        builder.push_slot_always::<WIPOffset<_>>(20, warnings);
        builder.end_table(table)
    };

    builder.finish(root, None);
    builder.finished_data().to_vec()
}

/// # Errors
///
/// Returns an error if any generated file or directory cannot be written.
///
/// # Panics
///
/// Panics if platform configs are missing when the corresponding platform plan
/// exists, or if schema files lack the expected generated prefix.
#[expect(
    clippy::too_many_lines,
    reason = "sequential file-write orchestration that is clearest kept in one function"
)]
pub fn emit_host_tree(repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<Vec<Utf8PathBuf>> {
    write_file(
        &repo_root.join(&plan.schema.aggregate),
        &render_aggregate_schema(plan),
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
        write_file(
            &repo_root.join(&ios.generated_root).join("BUILD.bazel"),
            &render_ios_build_file(
                &plan.app,
                &plan.modules,
                plan.ios_config
                    .as_ref()
                    .expect("ios config should exist when ios output exists"),
            )?,
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("Info.generated.plist"),
            &render_ios_plist(
                &plan.app,
                plan.ios_config
                    .as_ref()
                    .expect("ios config should exist when ios output exists"),
            ),
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
            &render_swift_scene_delegate(&plan.app),
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("AtomBindings.swift"),
            &render_swift_bindings(&plan.modules),
        )?;
        write_file(
            &repo_root.join(&ios.generated_root).join("main.swift"),
            &render_swift_main(),
        )?;
    }

    if let Some(android) = &plan.android {
        write_file(
            &repo_root.join(&android.generated_root).join("BUILD.bazel"),
            &render_android_build_file(
                &plan.app,
                &plan.modules,
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            )?,
        )?;
        write_file(
            &repo_root
                .join(&android.generated_root)
                .join("AndroidManifest.generated.xml"),
            &render_android_manifest_xml(
                &plan.app,
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            ),
        )?;
        let package_dir = android.files[2]
            .parent()
            .expect("android application file parent")
            .to_owned();
        write_file(
            &repo_root.join(&package_dir).join("AtomApplication.kt"),
            &render_kotlin_application(
                &plan.app,
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            ),
        )?;
        write_file(
            &repo_root.join(&package_dir).join("AtomBindings.kt"),
            &render_kotlin_bindings(
                &plan.modules,
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            ),
        )?;
        write_file(
            &repo_root.join(&package_dir).join("MainActivity.kt"),
            &render_kotlin_main_activity(
                &plan.app,
                &android.generated_root,
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            ),
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

fn create_prebuild_app<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    name: WIPOffset<&'a str>,
    slug: WIPOffset<&'a str>,
    entry_target: WIPOffset<&'a str>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, name);
    builder.push_slot_always::<WIPOffset<_>>(6, slug);
    builder.push_slot_always::<WIPOffset<_>>(8, entry_target);
    builder.end_table(table)
}

fn create_prebuild_module<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    id: WIPOffset<&'a str>,
    init_order: u32,
    target_label: WIPOffset<&'a str>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, id);
    builder.push_slot::<u32>(6, init_order, 0);
    builder.push_slot_always::<WIPOffset<_>>(8, target_label);
    builder.end_table(table)
}

fn create_prebuild_platform<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    generated_root: WIPOffset<&'a str>,
    target: WIPOffset<&'a str>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, generated_root);
    builder.push_slot_always::<WIPOffset<_>>(6, target);
    builder.end_table(table)
}

fn create_prebuild_schema<'a, T>(
    builder: &mut FlatBufferBuilder<'a>,
    aggregate: WIPOffset<&'a str>,
    modules: WIPOffset<T>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, aggregate);
    builder.push_slot_always::<WIPOffset<_>>(6, modules);
    builder.end_table(table)
}

fn build_ios_plan(app: &AppConfig, build: &BuildConfig, _ios: &IosConfig) -> PlatformPlan {
    let generated_root = build.generated_root.join("ios").join(&app.slug);
    let files = vec![
        generated_root.join("BUILD.bazel"),
        generated_root.join("Info.generated.plist"),
        generated_root.join("LaunchScreen.storyboard"),
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

fn normalize_schema_output(schema_file: &Utf8Path) -> Utf8PathBuf {
    let value = schema_file.as_str();
    if let Some(stripped) = value.strip_prefix("schema/") {
        return Utf8PathBuf::from(stripped);
    }
    if let Some((_, stripped)) = value.rsplit_once("/schema/") {
        return Utf8PathBuf::from(stripped);
    }
    schema_file.to_owned()
}

fn deep_merge_map(target: &mut JsonMap, source: &JsonMap, context: &str) -> AtomResult<()> {
    for (key, source_value) in source {
        match target.get_mut(key) {
            None => {
                target.insert(key.clone(), source_value.clone());
            }
            Some(target_value) => {
                deep_merge_value(target_value, source_value, &format!("{context}.{key}"))?;
            }
        }
    }
    Ok(())
}

fn deep_merge_value(target: &mut Value, source: &Value, path: &str) -> AtomResult<()> {
    match (target, source) {
        (Value::Object(target_map), Value::Object(source_map)) => {
            deep_merge_map(target_map, source_map, path)
        }
        (target_value, source_value) if target_value == source_value => Ok(()),
        _ => Err(AtomError::with_path(
            AtomErrorCode::CngConflict,
            "conflicting scalar values during CNG merge",
            path,
        )),
    }
}

fn render_aggregate_schema(plan: &GenerationPlan) -> String {
    let mut contents = String::new();
    for schema_file in &plan.schema.modules {
        let relative = schema_file
            .strip_prefix(plan.build.generated_root.join("schema"))
            .expect("schema file should live under generated schema root");
        let _ = writeln!(contents, "include \"{}\";", relative.as_str());
    }
    if !contents.is_empty() {
        contents.push('\n');
    }
    contents.push_str("namespace atom;\n\n");
    contents.push_str("table AtomAppConfig {\n");
    contents.push_str("  name: string;\n");
    contents.push_str("  slug: string;\n");
    contents.push_str("  entry_target: string;\n");
    contents.push_str("}\n");
    contents
}

fn render_ios_build_file(
    app: &AppConfig,
    modules: &[ResolvedModule],
    ios: &IosConfig,
) -> AtomResult<String> {
    let support_module = swift_support_module_name(app);
    let mut contents = String::from(
        "# Generated by atom. Do not edit.\n\
load(\"@build_bazel_rules_apple//apple:ios.bzl\", \"ios_application\")\n\
load(\"@build_bazel_rules_swift//swift:swift.bzl\", \"swift_library\")\n\n",
    );
    contents.push_str("swift_library(\n");
    contents.push_str("    name = \"generated_swift\",\n");
    contents.push_str("    srcs = [\n");
    contents.push_str("        \"main.swift\",\n");
    contents.push_str("        \"AtomAppDelegate.swift\",\n");
    contents.push_str("        \"SceneDelegate.swift\",\n");
    contents.push_str("        \"AtomBindings.swift\",\n");
    for module in modules {
        let label = metadata_target(&module.request.target_label, "_ios_srcs")?;
        let _ = writeln!(contents, "        \"{label}\",");
    }
    contents.push_str("    ],\n");
    let _ = writeln!(contents, "    module_name = \"{support_module}\",");
    let _ = writeln!(
        contents,
        "    deps = [\"//crates/atom-runtime:atom-runtime-swift-bridge\"],"
    );
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n\n");
    contents.push_str("ios_application(\n");
    contents.push_str("    name = \"app\",\n");
    let _ = writeln!(
        contents,
        "    bundle_id = \"{}\",",
        ios.bundle_id.as_deref().unwrap_or_default()
    );
    contents.push_str("    families = [\"iphone\", \"ipad\"],\n");
    contents.push_str("    infoplists = [\"Info.generated.plist\"],\n");
    contents.push_str("    resources = [\"LaunchScreen.storyboard\"],\n");
    let _ = writeln!(
        contents,
        "    minimum_os_version = \"{}\",",
        ios.deployment_target.as_deref().unwrap_or_default()
    );
    contents.push_str("    deps = [\":generated_swift\"],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n");
    Ok(contents)
}

fn render_android_build_file(
    app: &AppConfig,
    modules: &[ResolvedModule],
    android: &AndroidConfig,
) -> AtomResult<String> {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    let package_dir = kotlin_package_dir(package_name);
    let jni_prefix = jni_prefix(package_name);
    let source_root = Utf8PathBuf::from("src/main/kotlin").join(package_dir);
    let mut contents = String::from(
        "# Generated by atom. Do not edit.\n\
load(\"@rules_android//android:rules.bzl\", \"android_binary\")\n\
load(\"@rules_rust//rust:defs.bzl\", \"rust_shared_library\")\n\
load(\"@rules_kotlin//kotlin:jvm.bzl\", \"kt_jvm_library\")\n\n",
    );
    contents.push_str("rust_shared_library(\n");
    contents.push_str("    name = \"atom_runtime_jni\",\n");
    contents.push_str("    srcs = [\"//templates/atom_runtime_jni:lib.rs\"],\n");
    contents.push_str("    crate_name = \"atom_runtime_jni\",\n");
    contents.push_str("    edition = \"2024\",\n");
    contents.push_str("    rustc_env = {\n");
    let _ = writeln!(
        contents,
        "        \"ATOM_JNI_INIT_EXPORT\": \"{jni_prefix}_initRuntime\","
    );
    let _ = writeln!(
        contents,
        "        \"ATOM_JNI_LIFECYCLE_EXPORT\": \"{jni_prefix}_handleLifecycle\","
    );
    let _ = writeln!(
        contents,
        "        \"ATOM_JNI_SHUTDOWN_EXPORT\": \"{jni_prefix}_shutdown\","
    );
    contents.push_str("    },\n");
    contents.push_str("    visibility = [\"//visibility:private\"],\n");
    contents.push_str("    deps = [\n");
    contents.push_str("        \"//crates/atom-ffi\",\n");
    contents.push_str("        \"//crates/atom-runtime\",\n");
    let _ = writeln!(contents, "        \"{}\",", app.entry_crate_label);
    contents.push_str("    ],\n");
    contents.push_str(")\n\n");
    contents.push_str("kt_jvm_library(\n");
    contents.push_str("    name = \"generated_kotlin\",\n");
    contents.push_str("    srcs = [\n");
    let _ = writeln!(
        contents,
        "        \"{}/AtomApplication.kt\",",
        source_root.as_str()
    );
    let _ = writeln!(
        contents,
        "        \"{}/AtomBindings.kt\",",
        source_root.as_str()
    );
    let _ = writeln!(
        contents,
        "        \"{}/MainActivity.kt\",",
        source_root.as_str()
    );
    for module in modules {
        let label = metadata_target(&module.request.target_label, "_android_srcs")?;
        let _ = writeln!(contents, "        \"{label}\",");
    }
    contents.push_str("    ],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n\n");
    contents.push_str("android_binary(\n");
    contents.push_str("    name = \"app\",\n");
    contents.push_str("    manifest = \"AndroidManifest.generated.xml\",\n");
    let _ = writeln!(contents, "    custom_package = \"{package_name}\",");
    contents.push_str("    deps = [\":generated_kotlin\"],\n");
    contents.push_str("    data = [\":atom_runtime_jni\"],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n");
    Ok(contents)
}

fn render_ios_plist(app: &AppConfig, ios: &IosConfig) -> String {
    let support_module = swift_support_module_name(app);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>{}</string>
  <key>CFBundleDisplayName</key>
  <string>{}</string>
  <key>CFBundleIdentifier</key>
  <string>{}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSRequiresIPhoneOS</key>
  <true/>
  <key>MinimumOSVersion</key>
  <string>{}</string>
  <key>UIApplicationSupportsIndirectInputEvents</key>
  <true/>
  <key>UIApplicationSceneManifest</key>
  <dict>
    <key>UIApplicationSupportsMultipleScenes</key>
    <false/>
    <key>UISceneConfigurations</key>
    <dict>
      <key>UIWindowSceneSessionRoleApplication</key>
      <array>
        <dict>
          <key>UISceneConfigurationName</key>
          <string>Default Configuration</string>
          <key>UISceneDelegateClassName</key>
          <string>{}.AtomSceneDelegate</string>
        </dict>
      </array>
    </dict>
  </dict>
  <key>UIMainStoryboardFile</key>
  <string></string>
  <key>UILaunchStoryboardName</key>
  <string>LaunchScreen.storyboard</string>
  <key>UISupportedInterfaceOrientations</key>
  <array>
    <string>UIInterfaceOrientationPortrait</string>
  </array>
</dict>
</plist>
"#,
        app.slug,
        app.name,
        ios.bundle_id.as_deref().unwrap_or_default(),
        ios.deployment_target.as_deref().unwrap_or_default(),
        support_module
    )
}

fn render_ios_launch_storyboard() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<document type="com.apple.InterfaceBuilder3.CocoaTouch.Storyboard.XIB" version="3.0" toolsVersion="24127" targetRuntime="iOS.CocoaTouch" propertyAccessControl="none" useAutolayout="YES" launchScreen="YES" useTraitCollections="YES" useSafeAreas="YES" colorMatched="YES" initialViewController="01J-lp-oVM">
    <device id="retina6_1" orientation="portrait" appearance="light"/>
    <dependencies>
        <plugIn identifier="com.apple.InterfaceBuilder.IBCocoaTouchPlugin" version="24063"/>
        <capability name="Safe area layout guides" minToolsVersion="9.0"/>
        <capability name="System colors in document resources" minToolsVersion="11.0"/>
        <capability name="documents saved in the Xcode 8 format" minToolsVersion="8.0"/>
    </dependencies>
    <scenes>
        <scene sceneID="EHf-IW-A2E">
            <objects>
                <viewController id="01J-lp-oVM" sceneMemberID="viewController">
                    <view key="view" contentMode="scaleToFill" id="Ze5-6b-2t3">
                        <rect key="frame" x="0.0" y="0.0" width="414" height="896"/>
                        <autoresizingMask key="autoresizingMask" widthSizable="YES" heightSizable="YES"/>
                        <viewLayoutGuide key="safeArea" id="safe"/>
                        <color key="backgroundColor" systemColor="systemBackgroundColor"/>
                    </view>
                </viewController>
                <placeholder placeholderIdentifier="IBFirstResponder" id="iYj-Kq-Ea1" userLabel="First Responder" sceneMemberID="firstResponder"/>
            </objects>
            <point key="canvasLocation" x="53" y="375"/>
        </scene>
    </scenes>
    <resources>
        <systemColor name="systemBackgroundColor">
            <color white="1" alpha="1" colorSpace="custom" customColorSpace="genericGamma22GrayColorSpace"/>
        </systemColor>
    </resources>
</document>
"#
    .to_owned()
}

fn render_swift_app_delegate(_app: &AppConfig) -> String {
    format!(
        "import AtomRuntimeBridge\nimport UIKit\n\nfinal class AtomAppDelegate: NSObject, UIApplicationDelegate {{\n    private var handle: AtomRuntimeHandle = 0\n\n    func application(\n        _ application: UIApplication,\n        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil\n    ) -> Bool {{\n        true\n    }}\n\n    func application(\n        _ application: UIApplication,\n        configurationForConnecting connectingSceneSession: UISceneSession,\n        options: UIScene.ConnectionOptions\n    ) -> UISceneConfiguration {{\n        let configuration = UISceneConfiguration(\n            name: \"Default Configuration\",\n            sessionRole: connectingSceneSession.role\n        )\n        configuration.delegateClass = AtomSceneDelegate.self\n        return configuration\n    }}\n\n    func initializeRuntime() {{\n        guard handle == 0 else {{\n            return\n        }}\n        do {{\n            var handle: AtomRuntimeHandle = 0\n            var errorBuffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)\n            let status = atom_app_init(AtomSlice(ptr: nil, len: 0), &handle, &errorBuffer)\n            defer {{ freeBuffer(&errorBuffer) }}\n            try ensureSuccess(status, action: \"atom_app_init\")\n            self.handle = handle\n        }} catch {{\n            logError(error, action: \"launch\")\n        }}\n    }}\n\n    func sendLifecycle(_ event: UInt32, action: String) {{\n        guard handle != 0 else {{\n            return\n        }}\n        do {{\n            var errorBuffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)\n            let status = atom_app_handle_lifecycle(handle, event, &errorBuffer)\n            defer {{ freeBuffer(&errorBuffer) }}\n            try ensureSuccess(status, action: action)\n        }} catch {{\n            logError(error, action: action)\n        }}\n    }}\n\n    func shutdownRuntime() {{\n        guard handle != 0 else {{\n            return\n        }}\n        atom_app_shutdown(handle)\n        handle = 0\n    }}\n\n    private func ensureSuccess(_ status: Int32, action: String) throws {{\n        if status == 0 {{\n            return\n        }}\n        throw NSError(domain: \"AtomRuntime\", code: Int(status), userInfo: [NSLocalizedDescriptionKey: \"\\(action) failed with status \\(status)\"])\n    }}\n\n    private func freeBuffer(_ buffer: inout AtomOwnedBuffer) {{\n        guard buffer.ptr != nil else {{\n            return\n        }}\n        atom_buffer_free(buffer)\n        buffer = AtomOwnedBuffer(ptr: nil, len: 0, cap: 0)\n    }}\n\n    private func logError(_ error: Error, action: String) {{\n        NSLog(\"Atom iOS host %@ failed: %@\", action, String(describing: error))\n    }}\n}}\n"
    )
}

fn render_swift_scene_delegate(app: &AppConfig) -> String {
    format!(
        "import SwiftUI\nimport UIKit\n\nfinal class AtomSceneDelegate: UIResponder, UIWindowSceneDelegate {{\n    var window: UIWindow?\n\n    func scene(\n        _ scene: UIScene,\n        willConnectTo session: UISceneSession,\n        options connectionOptions: UIScene.ConnectionOptions\n    ) {{\n        guard let windowScene = scene as? UIWindowScene else {{\n            return\n        }}\n        let window = UIWindow(windowScene: windowScene)\n        window.frame = windowScene.coordinateSpace.bounds\n        window.backgroundColor = .systemBackground\n        window.rootViewController = UIHostingController(rootView: AtomRootView())\n        self.window = window\n        window.makeKeyAndVisible()\n        runtimeDelegate?.initializeRuntime()\n    }}\n\n    func sceneDidEnterBackground(_ scene: UIScene) {{\n        runtimeDelegate?.sendLifecycle(2, action: \"background\")\n    }}\n\n    func sceneWillEnterForeground(_ scene: UIScene) {{\n        runtimeDelegate?.sendLifecycle(1, action: \"foreground\")\n    }}\n\n    func sceneDidDisconnect(_ scene: UIScene) {{\n        runtimeDelegate?.sendLifecycle(5, action: \"terminate\")\n        runtimeDelegate?.shutdownRuntime()\n    }}\n\n    private var runtimeDelegate: AtomAppDelegate? {{\n        UIApplication.shared.delegate as? AtomAppDelegate\n    }}\n}}\n\nprivate struct AtomRootView: View {{\n    var body: some View {{\n        ZStack {{\n            Color(uiColor: .systemBackground)\n                .ignoresSafeArea()\n            VStack(spacing: 12) {{\n                Text(\"{name}\")\n                    .font(.title2.weight(.semibold))\n                Text(\"{slug}\")\n                    .font(.footnote.monospaced())\n                    .foregroundStyle(.secondary)\n                Text(AtomBindings.moduleSummary)\n                    .font(.footnote)\n                    .foregroundStyle(.secondary)\n                    .multilineTextAlignment(.center)\n                    .padding(.horizontal, 24)\n            }}\n            .frame(maxWidth: .infinity, maxHeight: .infinity)\n        }}\n    }}\n}}\n",
        name = app.name,
        slug = app.slug
    )
}

fn render_swift_main() -> String {
    "import UIKit\n\nUIApplicationMain(\n    CommandLine.argc,\n    CommandLine.unsafeArgv,\n    nil,\n    NSStringFromClass(AtomAppDelegate.self)\n)\n"
        .to_owned()
}

fn render_swift_bindings(modules: &[ResolvedModule]) -> String {
    let mut contents = String::from(
        "import Foundation\n\npublic enum AtomBindings {\n    public static let modules = [\n",
    );
    for module in modules {
        let _ = writeln!(contents, "        \"{}\",", module.manifest.id);
    }
    contents.push_str("    ]\n\n    public static var moduleSummary: String {\n        modules.joined(separator: \", \")\n    }\n}\n");
    contents
}

fn render_android_manifest_xml(app: &AppConfig, android: &AndroidConfig) -> String {
    format!(
        r#"<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="{}">
  <uses-sdk android:minSdkVersion="{}" android:targetSdkVersion="{}" />
  <application android:label="{}" android:name=".AtomApplication">
    <activity android:name=".MainActivity" android:exported="true" />
  </application>
</manifest>
"#,
        android.application_id.as_deref().unwrap_or_default(),
        android.min_sdk.unwrap_or_default(),
        android.target_sdk.unwrap_or_default(),
        app.name
    )
}

fn render_kotlin_application(app: &AppConfig, android: &AndroidConfig) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    format!(
        "package {package_name}\n\nclass AtomApplication {{\n    val name: String = \"{name}\"\n    val slug: String = \"{slug}\"\n}}\n",
        name = app.name,
        slug = app.slug
    )
}

fn render_kotlin_bindings(modules: &[ResolvedModule], android: &AndroidConfig) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    let mut contents =
        format!("package {package_name}\n\nobject AtomBindings {{\n    val modules = listOf(\n");
    for module in modules {
        let _ = writeln!(contents, "        \"{}\",", module.manifest.id);
    }
    contents.push_str("    )\n\n    val moduleSummary: String\n        get() = modules.joinToString(separator = \", \")\n}\n");
    contents
}

fn render_kotlin_main_activity(
    app: &AppConfig,
    generated_root: &Utf8Path,
    android: &AndroidConfig,
) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    format!(
        "package {package_name}\n\nimport java.nio.file.Files\nimport java.nio.file.Paths\n\nclass MainActivity {{\n    fun run() {{\n        val application = AtomApplication()\n        val handle = AtomRuntimeBridge.initRuntime()\n        check(handle != 0L) {{ \"atom_app_init failed\" }}\n        try {{\n            println(\"Booting {name} ({slug}) on Android\")\n            println(\"Modules: ${{AtomBindings.moduleSummary}}\")\n            check(AtomRuntimeBridge.handleLifecycle(handle, 2) == 0) {{ \"background transition failed\" }}\n            check(AtomRuntimeBridge.handleLifecycle(handle, 3) == 0) {{ \"suspend transition failed\" }}\n            check(AtomRuntimeBridge.handleLifecycle(handle, 4) == 0) {{ \"resume transition failed\" }}\n            check(AtomRuntimeBridge.handleLifecycle(handle, 5) == 0) {{ \"terminate transition failed\" }}\n            println(\"Started ${{application.name}} with slug ${{application.slug}}\")\n        }} finally {{\n            AtomRuntimeBridge.shutdown(handle)\n        }}\n    }}\n}}\n\nprivate object AtomRuntimeBridge {{\n    init {{\n        System.load(resolveLibraryPath())\n    }}\n\n    external fun initRuntime(): Long\n    external fun handleLifecycle(handle: Long, event: Int): Int\n    external fun shutdown(handle: Long)\n\n    private fun resolveLibraryPath(): String {{\n        val runfilesDir = System.getenv(\"JAVA_RUNFILES\")\n            ?: System.getenv(\"RUNFILES_DIR\")\n            ?: error(\"Bazel runfiles directory is not available\")\n        val libraryName = System.mapLibraryName(\"atom_runtime_jni\")\n        val candidates = listOf(\"_main\", \"atom\").map {{ workspace ->\n            Paths.get(runfilesDir, workspace, \"{generated_root}\", libraryName)\n        }}\n        return candidates.firstOrNull(Files::exists)?.toString()\n            ?: error(\"Unable to locate atom_runtime_jni in Bazel runfiles\")\n    }}\n}}\n\nfun main() {{\n    MainActivity().run()\n}}\n",
        name = app.name,
        slug = app.slug,
        generated_root = generated_root.as_str()
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

fn swift_support_module_name(app: &AppConfig) -> String {
    format!("atom_{}_support", app.slug.replace('-', "_"))
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

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_manifest::{
        AndroidConfig, AppConfig, BuildConfig, IosConfig, ModuleRequest, NormalizedManifest,
    };
    use atom_modules::{JsonMap, MethodSpec, ModuleKind, ModuleManifest, ResolvedModule};
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{build_generation_plan, emit_host_tree, render_prebuild_plan};

    fn write_fixture(root: &Utf8PathBuf) -> (NormalizedManifest, Vec<ResolvedModule>) {
        fs::create_dir_all(root.join("modules/device_info/schema")).expect("module dir");
        fs::write(
            root.join("modules/device_info/schema/device_info.fbs"),
            "namespace atom.device_info;\n",
        )
        .expect("schema");

        let manifest = NormalizedManifest {
            repo_root: root.clone(),
            target_label: "//apps/hello_atom:hello_atom".to_owned(),
            metadata_path: root.join("bazel-out/hello_atom.atom.app.json"),
            app: AppConfig {
                name: "Hello Atom".to_owned(),
                slug: "hello-atom".to_owned(),
                entry_crate_label: "//apps/hello_atom:hello_atom".to_owned(),
            },
            ios: IosConfig {
                enabled: true,
                bundle_id: Some("build.atom.hello".to_owned()),
                deployment_target: Some("17.0".to_owned()),
            },
            android: AndroidConfig {
                enabled: true,
                application_id: Some("build.atom.hello".to_owned()),
                min_sdk: Some(28),
                target_sdk: Some(35),
            },
            build: BuildConfig {
                generated_root: Utf8PathBuf::from("generated"),
                watch: false,
            },
            modules: vec![ModuleRequest {
                target_label: "//modules/device_info:device_info".to_owned(),
            }],
        };

        let modules = vec![ResolvedModule {
            request: ModuleRequest {
                target_label: "//modules/device_info:device_info".to_owned(),
            },
            metadata_path: root.join("bazel-out/device_info.atom.module.json"),
            manifest: ModuleManifest {
                kind: ModuleKind::Rust,
                target_label: "//modules/device_info:device_info".to_owned(),
                id: "device_info".to_owned(),
                depends_on: Vec::new(),
                schema_files: vec![Utf8PathBuf::from(
                    "modules/device_info/schema/device_info.fbs",
                )],
                methods: vec![MethodSpec {
                    name: "get".to_owned(),
                    request_table: "atom.device_info.GetDeviceInfoRequest".to_owned(),
                    response_table: "atom.device_info.GetDeviceInfoResponse".to_owned(),
                }],
                permissions: Vec::new(),
                plist: JsonMap::new(),
                android_manifest: JsonMap::new(),
                entitlements: JsonMap::new(),
                generated_sources: Vec::new(),
                init_priority: 0,
                ios_srcs: vec![Utf8PathBuf::from(
                    "modules/device_info/ios/DeviceInfoModule.swift",
                )],
                android_srcs: vec![Utf8PathBuf::from(
                    "modules/device_info/android/DeviceInfoModule.kt",
                )],
            },
            resolution_index: 0,
            layer: 0,
            init_order: 0,
        }];

        (manifest, modules)
    }

    #[test]
    fn plan_contains_required_generated_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules).expect("plan");

        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/schema/atom.fbs"))
        );
        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/ios/hello-atom/BUILD.bazel"))
        );
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/android/hello-atom/AndroidManifest.generated.xml"
        )));
        assert!(!render_prebuild_plan(&plan).is_empty());
    }

    #[test]
    fn emit_host_tree_writes_expected_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules).expect("plan");

        emit_host_tree(&root, &plan).expect("host tree");

        assert!(root.join("generated/schema/atom.fbs").exists());
        assert!(root.join("generated/ios/hello-atom/BUILD.bazel").exists());
        assert!(
            root.join("generated/android/hello-atom/BUILD.bazel")
                .exists()
        );
    }

    #[test]
    fn emit_host_tree_writes_phase_three_build_targets() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules).expect("plan");
        emit_host_tree(&root, &plan).expect("host tree");

        let ios_build =
            fs::read_to_string(root.join("generated/ios/hello-atom/BUILD.bazel")).expect("ios");
        let ios_plist =
            fs::read_to_string(root.join("generated/ios/hello-atom/Info.generated.plist"))
                .expect("ios plist");
        let ios_launch_storyboard =
            fs::read_to_string(root.join("generated/ios/hello-atom/LaunchScreen.storyboard"))
                .expect("ios launch storyboard");
        let swift_app_delegate =
            fs::read_to_string(root.join("generated/ios/hello-atom/AtomAppDelegate.swift"))
                .expect("swift app delegate");
        let swift_scene_delegate =
            fs::read_to_string(root.join("generated/ios/hello-atom/SceneDelegate.swift"))
                .expect("swift scene delegate");
        let swift_main = fs::read_to_string(root.join("generated/ios/hello-atom/main.swift"))
            .expect("swift main");
        let android_build =
            fs::read_to_string(root.join("generated/android/hello-atom/BUILD.bazel"))
                .expect("android build");
        let android_main =
            fs::read_to_string(root.join(
                "generated/android/hello-atom/src/main/kotlin/build/atom/hello/MainActivity.kt",
            ))
            .expect("android main");

        assert!(ios_build.contains("ios_application("));
        assert!(ios_build.contains("bundle_id = \"build.atom.hello\""));
        assert!(ios_build.contains("minimum_os_version = \"17.0\""));
        assert!(ios_build.contains("atom-runtime-swift-bridge"));
        assert!(!ios_build.contains("swift_binary("));
        assert!(ios_build.contains("resources = [\"LaunchScreen.storyboard\"]"));
        assert!(ios_plist.contains("<key>CFBundleShortVersionString</key>"));
        assert!(ios_plist.contains("<string>1.0</string>"));
        assert!(ios_plist.contains("<key>CFBundleVersion</key>"));
        assert!(ios_plist.contains("<string>1</string>"));
        assert!(ios_plist.contains("<key>UIApplicationSceneManifest</key>"));
        assert!(ios_plist.contains("<key>UILaunchStoryboardName</key>"));
        assert!(ios_plist.contains("<string>LaunchScreen.storyboard</string>"));
        assert!(ios_plist.contains("<key>UISceneDelegateClassName</key>"));
        assert!(ios_plist.contains("atom_hello_atom_support.AtomSceneDelegate"));
        assert!(ios_launch_storyboard.contains("launchScreen=\"YES\""));
        assert!(swift_app_delegate.contains("configurationForConnecting"));
        assert!(swift_main.contains("UIApplicationMain("));
        assert!(swift_main.contains("NSStringFromClass(AtomAppDelegate.self)"));
        assert!(swift_scene_delegate.contains("UIHostingController(rootView: AtomRootView())"));
        assert!(swift_scene_delegate.contains("Text(\"Hello Atom\")"));
        assert!(android_build.contains("rust_shared_library("));
        assert!(
            android_build
                .contains("load(\"@rules_android//android:rules.bzl\", \"android_binary\")")
        );
        assert!(android_build.contains("android_binary("));
        assert!(android_build.contains("manifest = \"AndroidManifest.generated.xml\""));
        assert!(android_build.contains("custom_package = \"build.atom.hello\""));
        assert!(!android_build.contains("java_binary("));
        assert!(!android_build.contains("AppEntry.kt"));
        assert!(android_main.contains("System.mapLibraryName(\"atom_runtime_jni\")"));
        assert!(
            !root
                .join("generated/android/hello-atom/src/main/kotlin/build/atom/hello/AppEntry.kt")
                .exists()
        );
    }
}
