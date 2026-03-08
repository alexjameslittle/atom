use std::collections::BTreeSet;
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
            module.init_order as u32,
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
            &render_ios_build_file(&plan.modules)?,
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
                .join("AtomAppDelegate.swift"),
            &render_swift_app_delegate(&plan.app),
        )?;
        write_file(
            &repo_root
                .join(&ios.generated_root)
                .join("AtomBindings.swift"),
            &render_swift_bindings(&plan.modules),
        )?;
        write_file(
            &repo_root.join(&ios.generated_root).join("main.swift"),
            &render_swift_main(&plan.app),
        )?;
    }

    if let Some(android) = &plan.android {
        write_file(
            &repo_root.join(&android.generated_root).join("BUILD.bazel"),
            &render_android_build_file(
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
                plan.android_config
                    .as_ref()
                    .expect("android config should exist when android output exists"),
            ),
        )?;
        write_file(
            &repo_root.join(&package_dir).join("AppEntry.kt"),
            &render_kotlin_app_entry(
                &plan.app,
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
        generated_root.join("AtomAppDelegate.swift"),
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
        source_root.join("AppEntry.kt"),
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
        contents.push_str(&format!("include \"{}\";\n", relative.as_str()));
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

fn render_ios_build_file(modules: &[ResolvedModule]) -> AtomResult<String> {
    let mut contents = String::from(
        "# Generated by atom. Do not edit.\n\
load(\"@build_bazel_rules_swift//swift:swift_binary.bzl\", \"swift_binary\")\n\
load(\"@build_bazel_rules_swift//swift:swift_library.bzl\", \"swift_library\")\n\n",
    );
    contents.push_str("swift_library(\n");
    contents.push_str("    name = \"generated_swift\",\n");
    contents.push_str("    srcs = [\n");
    contents.push_str("        \"AtomAppDelegate.swift\",\n");
    contents.push_str("        \"AtomBindings.swift\",\n");
    for module in modules {
        let label = metadata_target(&module.request.target_label, "_ios_srcs")?;
        contents.push_str(&format!("        \"{label}\",\n"));
    }
    contents.push_str("    ],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n\n");
    contents.push_str("swift_binary(\n");
    contents.push_str("    name = \"app\",\n");
    contents.push_str("    srcs = [\"main.swift\"],\n");
    contents.push_str("    deps = [\":generated_swift\"],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n");
    Ok(contents)
}

fn render_android_build_file(
    modules: &[ResolvedModule],
    android: &AndroidConfig,
) -> AtomResult<String> {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    let package_dir = kotlin_package_dir(package_name);
    let source_root = Utf8PathBuf::from("src/main/kotlin").join(package_dir);
    let mut contents = String::from(
        "# Generated by atom. Do not edit.\n\
load(\"@rules_kotlin//kotlin:jvm.bzl\", \"kt_jvm_binary\", \"kt_jvm_library\")\n\n",
    );
    contents.push_str("kt_jvm_library(\n");
    contents.push_str("    name = \"generated_kotlin\",\n");
    contents.push_str("    srcs = [\n");
    contents.push_str(&format!(
        "        \"{}/AtomApplication.kt\",\n",
        source_root.as_str()
    ));
    contents.push_str(&format!(
        "        \"{}/AtomBindings.kt\",\n",
        source_root.as_str()
    ));
    contents.push_str(&format!(
        "        \"{}/MainActivity.kt\",\n",
        source_root.as_str()
    ));
    for module in modules {
        let label = metadata_target(&module.request.target_label, "_android_srcs")?;
        contents.push_str(&format!("        \"{label}\",\n"));
    }
    contents.push_str("    ],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n\n");
    contents.push_str("kt_jvm_binary(\n");
    contents.push_str("    name = \"app\",\n");
    contents.push_str(&format!(
        "    main_class = \"{}.AppEntryKt\",\n",
        package_name
    ));
    contents.push_str("    srcs = [\n");
    contents.push_str(&format!(
        "        \"{}/AppEntry.kt\",\n",
        source_root.as_str()
    ));
    contents.push_str("    ],\n");
    contents.push_str("    deps = [\":generated_kotlin\"],\n");
    contents.push_str("    visibility = [\"//visibility:public\"],\n");
    contents.push_str(")\n");
    Ok(contents)
}

fn render_ios_plist(app: &AppConfig, ios: &IosConfig) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>{}</string>
  <key>CFBundleIdentifier</key>
  <string>{}</string>
  <key>MinimumOSVersion</key>
  <string>{}</string>
</dict>
</plist>
"#,
        app.name,
        ios.bundle_id.as_deref().unwrap_or_default(),
        ios.deployment_target.as_deref().unwrap_or_default()
    )
}

fn render_swift_app_delegate(app: &AppConfig) -> String {
    format!(
        "import Foundation\n\nfinal class AtomAppDelegate {{\n    static let slug = \"{}\"\n}}\n",
        app.slug
    )
}

fn render_swift_bindings(modules: &[ResolvedModule]) -> String {
    let mut contents =
        String::from("import Foundation\n\nstruct AtomBindings {\n    static let modules = [\n");
    for module in modules {
        contents.push_str(&format!("        \"{}\",\n", module.manifest.id));
    }
    contents.push_str("    ]\n}\n");
    contents
}

fn render_swift_main(app: &AppConfig) -> String {
    format!(
        "import Foundation\n\nprint(\"Booting {} ({})\")\n",
        app.name, app.slug
    )
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
        "package {}\n\nclass AtomApplication {{\n    val slug: String = \"{}\"\n}}\n",
        package_name, app.slug
    )
}

fn render_kotlin_bindings(modules: &[ResolvedModule], android: &AndroidConfig) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    let mut contents = format!(
        "package {}\n\nobject AtomBindings {{\n    val modules = listOf(\n",
        package_name
    );
    for module in modules {
        contents.push_str(&format!("        \"{}\",\n", module.manifest.id));
    }
    contents.push_str("    )\n}\n");
    contents
}

fn render_kotlin_main_activity(app: &AppConfig, android: &AndroidConfig) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    format!(
        "package {}\n\nclass MainActivity {{\n    fun title(): String = \"{}\"\n}}\n",
        package_name, app.name
    )
}

fn render_kotlin_app_entry(app: &AppConfig, android: &AndroidConfig) -> String {
    let package_name = android.application_id.as_deref().unwrap_or_default();
    format!(
        "package {}\n\nfun main() {{\n    println(\"Booting {} ({})\")\n}}\n",
        package_name, app.name, app.slug
    )
}

fn kotlin_package_dir(application_id: &str) -> Utf8PathBuf {
    Utf8PathBuf::from(application_id.replace('.', "/"))
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
}
