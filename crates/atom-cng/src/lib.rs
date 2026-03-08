mod android;
mod emit;
mod ios;
mod templates;

use std::collections::BTreeSet;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
use atom_modules::{JsonMap, ResolvedModule};
use camino::Utf8PathBuf;
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde_json::Value;

use crate::android::build_android_plan;
pub use crate::emit::emit_host_tree;
use crate::ios::build_ios_plan;

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

fn normalize_schema_output(schema_file: &camino::Utf8Path) -> Utf8PathBuf {
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

fn render_aggregate_schema(plan: &GenerationPlan) -> AtomResult<String> {
    let schema_includes: Vec<&str> = plan
        .schema
        .modules
        .iter()
        .map(|schema_file| {
            schema_file
                .strip_prefix(plan.build.generated_root.join("schema"))
                .expect("schema file should live under generated schema root")
                .as_str()
        })
        .collect();
    templates::render(
        "schema/atom.fbs",
        minijinja::context! {
            schema_includes,
        },
    )
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
