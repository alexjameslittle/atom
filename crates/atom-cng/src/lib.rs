mod android;
mod emit;
mod ios;
mod templates;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    AndroidConfig, AppConfig, BuildConfig, ConfigPluginRequest, FRAMEWORK_ATOM_API_LEVEL,
    FRAMEWORK_VERSION, IosConfig, NormalizedManifest,
};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde_json::{Value, json};

use crate::android::build_android_plan;
pub use crate::emit::emit_host_tree;
use crate::ios::build_ios_plan;

pub type ConfigPluginFactory = fn(&ConfigPluginRequest) -> AtomResult<Box<dyn ConfigPlugin>>;

pub trait ConfigPlugin: Send + Sync {
    fn id(&self) -> &str;

    /// # Errors
    ///
    /// Returns an error if the plugin configuration is invalid.
    fn validate(&self) -> AtomResult<()>;

    /// # Errors
    ///
    /// Returns an error if the plugin cannot produce valid iOS contributions.
    fn contribute_ios(&self, ctx: &ConfigPluginContext<'_>) -> AtomResult<PlatformContribution>;

    /// # Errors
    ///
    /// Returns an error if the plugin cannot produce valid Android contributions.
    fn contribute_android(&self, ctx: &ConfigPluginContext<'_>)
    -> AtomResult<PlatformContribution>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPluginContext<'a> {
    pub app: &'a AppConfig,
    pub repo_root: &'a Utf8Path,
    pub generated_root: &'a Utf8Path,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributedFile {
    pub source: FileSource,
    pub output: Utf8PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileSource {
    Copy(Utf8PathBuf),
    Content(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlatformContribution {
    pub files: Vec<ContributedFile>,
    pub plist_entries: JsonMap,
    pub android_manifest_entries: JsonMap,
    pub bazel_resources: Vec<String>,
    pub bazel_resource_globs: Vec<String>,
}

#[derive(Default)]
pub struct ConfigPluginRegistry {
    factories: BTreeMap<String, ConfigPluginFactory>,
}

impl ConfigPluginRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, id: &str, factory: ConfigPluginFactory) {
        self.factories.insert(id.to_owned(), factory);
    }

    fn instantiate(&self, entry: &ConfigPluginRequest) -> AtomResult<Box<dyn ConfigPlugin>> {
        let Some(factory) = self.factories.get(&entry.id) else {
            return Err(AtomError::with_path(
                AtomErrorCode::ExtensionIncompatible,
                format!("no config plugin is registered for id {}", entry.id),
                format!("config_plugins.{}.id", entry.id),
            ));
        };

        let plugin = factory(entry)?;
        if plugin.id() != entry.id {
            return Err(AtomError::with_path(
                AtomErrorCode::InternalBug,
                format!(
                    "config plugin registry returned id {} for requested id {}",
                    plugin.id(),
                    entry.id
                ),
                entry.target_label.as_str(),
            ));
        }
        Ok(plugin)
    }
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
    pub contributed_files: Vec<ContributedFile>,
    pub ios_resources: Vec<String>,
    pub ios_resource_globs: Vec<String>,
    pub android_resources: Vec<String>,
    pub ios: Option<PlatformPlan>,
    pub android: Option<PlatformPlan>,
    pub generated_files: Vec<Utf8PathBuf>,
    pub warnings: Vec<String>,
}

/// # Errors
///
/// Returns an error if compatibility validation or metadata merging fails.
#[expect(
    clippy::too_many_lines,
    reason = "generation planning merges module and plugin contributions in a single pass"
)]
pub fn build_generation_plan(
    manifest: &NormalizedManifest,
    modules: &[ResolvedModule],
    config_plugins: &ConfigPluginRegistry,
) -> AtomResult<GenerationPlan> {
    validate_extension_compatibility(manifest, modules)?;

    let mut permissions = BTreeSet::new();
    let mut plist = if manifest.ios.enabled {
        default_ios_plist(&manifest.app, &manifest.ios)
    } else {
        JsonMap::new()
    };
    let mut android_manifest = if manifest.android.enabled {
        default_android_manifest(&manifest.app, &manifest.android)
    } else {
        JsonMap::new()
    };
    let mut entitlements = JsonMap::new();
    let mut schema_outputs = Vec::new();
    let mut contributed_files = Vec::new();
    let mut ios_resources = Vec::new();
    let mut ios_resource_globs = Vec::new();
    let mut android_resources = Vec::new();
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

    let plugin_ctx = ConfigPluginContext {
        app: &manifest.app,
        repo_root: &manifest.repo_root,
        generated_root: &manifest.build.generated_root,
    };
    for entry in &manifest.config_plugins {
        let plugin = config_plugins.instantiate(entry)?;
        plugin.validate()?;

        if manifest.ios.enabled {
            let contribution = plugin.contribute_ios(&plugin_ctx)?;
            deep_merge_map(
                &mut plist,
                &contribution.plist_entries,
                &format!("config_plugins.{}.plist", entry.id),
            )?;
            contributed_files.extend(contribution.files);
            ios_resources.extend(contribution.bazel_resources);
            ios_resource_globs.extend(contribution.bazel_resource_globs);
        }
        if manifest.android.enabled {
            let contribution = plugin.contribute_android(&plugin_ctx)?;
            deep_merge_map(
                &mut android_manifest,
                &contribution.android_manifest_entries,
                &format!("config_plugins.{}.android_manifest", entry.id),
            )?;
            contributed_files.extend(contribution.files);
            android_resources.extend(contribution.bazel_resources);
        }
    }

    if manifest.android.enabled && manifest.app.automation_fixture {
        ensure_android_permission(&mut android_manifest, "android.permission.INTERNET");
        ensure_android_application_attribute(
            &mut android_manifest,
            "@android:usesCleartextTraffic",
            Value::Bool(true),
        );
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
    generated_files.extend(contributed_files.iter().map(|file| file.output.clone()));

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
        contributed_files,
        ios_resources,
        ios_resource_globs,
        android_resources,
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

pub(crate) fn render_plist_document(plist: &JsonMap) -> AtomResult<String> {
    let mut output = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n",
    );
    render_plist_value(&mut output, &Value::Object(plist.clone()), 0)?;
    output.push_str("</plist>\n");
    Ok(output)
}

pub(crate) fn render_android_manifest_document(
    package_name: &str,
    manifest: &JsonMap,
) -> AtomResult<String> {
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
                            ios::swift_support_module_name(app)
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

fn validate_extension_compatibility(
    manifest: &NormalizedManifest,
    modules: &[ResolvedModule],
) -> AtomResult<()> {
    for module in modules {
        validate_extension(
            module.manifest.target_label.as_str(),
            module.manifest.atom_api_level,
            module.manifest.min_atom_version.as_deref(),
            module.manifest.ios_min_deployment_target.as_deref(),
            module.manifest.android_min_sdk,
            manifest,
        )?;
    }
    for plugin in &manifest.config_plugins {
        validate_extension(
            plugin.target_label.as_str(),
            plugin.atom_api_level,
            plugin.min_atom_version.as_deref(),
            plugin.ios_min_deployment_target.as_deref(),
            plugin.android_min_sdk,
            manifest,
        )?;
    }
    Ok(())
}

fn validate_extension(
    target_label: &str,
    atom_api_level: u32,
    min_atom_version: Option<&str>,
    ios_min_deployment_target: Option<&str>,
    android_min_sdk: Option<u32>,
    manifest: &NormalizedManifest,
) -> AtomResult<()> {
    if atom_api_level != FRAMEWORK_ATOM_API_LEVEL {
        return Err(extension_compatibility_error(
            target_label,
            "atom_api_level",
            format!(
                "extension requires atom_api_level {atom_api_level}, framework supports {FRAMEWORK_ATOM_API_LEVEL}"
            ),
        ));
    }

    if let Some(min_atom_version) = min_atom_version
        && compare_semver(FRAMEWORK_VERSION, min_atom_version)? == Ordering::Less
    {
        return Err(extension_compatibility_error(
            target_label,
            "min_atom_version",
            format!(
                "extension requires Atom version {min_atom_version}, framework is {FRAMEWORK_VERSION}"
            ),
        ));
    }

    if let (Some(current), Some(required)) = (
        manifest.ios.deployment_target.as_deref(),
        ios_min_deployment_target,
    ) && compare_deployment_target(current, required)? == Ordering::Less
    {
        return Err(extension_compatibility_error(
            target_label,
            "ios_min_deployment_target",
            format!(
                "extension requires iOS deployment target {required}, app is configured for {current}"
            ),
        ));
    }

    if let (Some(current), Some(required)) = (manifest.android.min_sdk, android_min_sdk)
        && current < required
    {
        return Err(extension_compatibility_error(
            target_label,
            "android_min_sdk",
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

fn compare_deployment_target(current: &str, required: &str) -> AtomResult<Ordering> {
    let current = parse_deployment_target(current)?;
    let required = parse_deployment_target(required)?;
    Ok(current.cmp(&required))
}

fn compare_semver(current: &str, required: &str) -> AtomResult<Ordering> {
    let current = parse_semver(current)?;
    let required = parse_semver(required)?;
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

fn parse_semver(value: &str) -> AtomResult<(u32, u32, u32)> {
    let mut components = value.split('.');
    match (
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ) {
        (Some(major), Some(minor), Some(patch), None) => Ok((
            parse_u32_component(major, "semver")?,
            parse_u32_component(minor, "semver")?,
            parse_u32_component(patch, "semver")?,
        )),
        _ => Err(AtomError::new(
            AtomErrorCode::InternalBug,
            format!("invalid semver format: {value}"),
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

fn object_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map,
        _ => JsonMap::new(),
    }
}

fn ensure_android_permission(manifest: &mut JsonMap, permission: &str) {
    let key = "uses-permission".to_owned();
    let entry = json!({
        "@android:name": permission,
    });
    let current = manifest.remove(&key);
    let next = match current {
        None => Value::Array(vec![entry]),
        Some(Value::Array(mut values)) => {
            if !android_permission_present(&values, permission) {
                values.push(entry);
            }
            Value::Array(values)
        }
        Some(Value::Object(map)) => {
            let mut values = vec![Value::Object(map)];
            if !android_permission_present(&values, permission) {
                values.push(entry);
            }
            Value::Array(values)
        }
        Some(other) => other,
    };
    manifest.insert(key, next);
}

fn android_permission_present(values: &[Value], permission: &str) -> bool {
    values.iter().any(|value| {
        value
            .as_object()
            .and_then(|map| map.get("@android:name"))
            .and_then(Value::as_str)
            == Some(permission)
    })
}

fn ensure_android_application_attribute(manifest: &mut JsonMap, key: &str, value: Value) {
    if let Some(Value::Object(application)) = manifest.get_mut("application") {
        application.insert(key.to_owned(), value);
    }
}

fn render_plist_value(output: &mut String, value: &Value, indent: usize) -> AtomResult<()> {
    let prefix = "  ".repeat(indent);
    match value {
        Value::Object(map) => {
            writeln!(output, "{prefix}<dict>").expect("write to string");
            for (key, entry) in map {
                writeln!(output, "{prefix}  <key>{}</key>", xml_escape(key)).expect("write");
                render_plist_value(output, entry, indent + 1)?;
            }
            writeln!(output, "{prefix}</dict>").expect("write to string");
        }
        Value::Array(values) => {
            writeln!(output, "{prefix}<array>").expect("write to string");
            for entry in values {
                render_plist_value(output, entry, indent + 1)?;
            }
            writeln!(output, "{prefix}</array>").expect("write to string");
        }
        Value::String(value) => {
            writeln!(output, "{prefix}<string>{}</string>", xml_escape(value)).expect("write");
        }
        Value::Bool(true) => {
            writeln!(output, "{prefix}<true/>").expect("write to string");
        }
        Value::Bool(false) => {
            writeln!(output, "{prefix}<false/>").expect("write to string");
        }
        Value::Number(number) => {
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

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_manifest::{
        AndroidConfig, AppConfig, BuildConfig, ConfigPluginRequest, IosConfig, JsonMap,
        ModuleRequest, NormalizedManifest,
    };
    use atom_modules::{MethodSpec, ModuleKind, ModuleManifest, ResolvedModule};
    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        ConfigPlugin, ConfigPluginContext, ConfigPluginRegistry, ContributedFile, FileSource,
        PlatformContribution, build_generation_plan, emit_host_tree, object_from_value,
        render_prebuild_plan,
    };

    struct FixturePlugin;

    impl ConfigPlugin for FixturePlugin {
        fn id(&self) -> &str {
            "fixture_plugin"
        }

        fn validate(&self) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn contribute_ios(
            &self,
            ctx: &ConfigPluginContext<'_>,
        ) -> atom_ffi::AtomResult<PlatformContribution> {
            Ok(PlatformContribution {
                files: vec![ContributedFile {
                    source: FileSource::Copy(Utf8PathBuf::from("assets/AppIcon.icon")),
                    output: ctx
                        .generated_root
                        .join("ios")
                        .join(&ctx.app.slug)
                        .join("resources")
                        .join("AppIcon.icon"),
                }],
                plist_entries: object_from_value(json!({
                    "CFBundleIconName": "AppIcon"
                })),
                android_manifest_entries: JsonMap::new(),
                bazel_resources: Vec::new(),
                bazel_resource_globs: vec!["resources/AppIcon.icon/**".to_owned()],
            })
        }

        fn contribute_android(
            &self,
            ctx: &ConfigPluginContext<'_>,
        ) -> atom_ffi::AtomResult<PlatformContribution> {
            Ok(PlatformContribution {
                files: vec![ContributedFile {
                    source: FileSource::Copy(Utf8PathBuf::from("assets/ic_launcher.png")),
                    output: ctx
                        .generated_root
                        .join("android")
                        .join(&ctx.app.slug)
                        .join("res/mipmap-xxxhdpi/ic_launcher.png"),
                }],
                plist_entries: JsonMap::new(),
                android_manifest_entries: object_from_value(json!({
                    "application": {
                        "@android:icon": "@mipmap/ic_launcher"
                    }
                })),
                bazel_resources: vec!["res/mipmap-xxxhdpi/ic_launcher.png".to_owned()],
                bazel_resource_globs: Vec::new(),
            })
        }
    }

    fn register_fixture_plugin(registry: &mut ConfigPluginRegistry) {
        registry.register("fixture_plugin", |_entry| Ok(Box::new(FixturePlugin)));
    }

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
                entry_crate_name: "hello_atom".to_owned(),
                automation_fixture: false,
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
            config_plugins: Vec::new(),
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
                atom_api_level: 1,
                min_atom_version: Some("0.1.0".to_owned()),
                ios_min_deployment_target: Some("17.0".to_owned()),
                android_min_sdk: Some(28),
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

    fn fixture_registry() -> ConfigPluginRegistry {
        ConfigPluginRegistry::default()
    }

    #[test]
    fn plan_contains_required_generated_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry()).expect("plan");

        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/schema/atom.fbs"))
        );
        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/ios/hello-atom/BUILD.bazel"))
        );
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/ios/hello-atom/atom_runtime_app_bridge.rs"
        )));
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/android/hello-atom/AndroidManifest.generated.xml"
        )));
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/android/hello-atom/atom_runtime_jni.rs"
        )));
        assert!(!render_prebuild_plan(&plan).is_empty());
    }

    #[test]
    fn emit_host_tree_writes_expected_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry()).expect("plan");

        emit_host_tree(&root, &plan).expect("host tree");

        assert!(root.join("generated/schema/atom.fbs").exists());
        assert!(root.join("generated/ios/hello-atom/BUILD.bazel").exists());
        assert!(
            root.join("generated/android/hello-atom/BUILD.bazel")
                .exists()
        );
    }

    #[test]
    fn emit_host_tree_writes_phase_five_build_targets() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = write_fixture(&root);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry()).expect("plan");
        emit_host_tree(&root, &plan).expect("host tree");

        let ios_build =
            fs::read_to_string(root.join("generated/ios/hello-atom/BUILD.bazel")).expect("ios");
        let ios_plist =
            fs::read_to_string(root.join("generated/ios/hello-atom/Info.generated.plist"))
                .expect("ios plist");
        let ios_launch_storyboard =
            fs::read_to_string(root.join("generated/ios/hello-atom/LaunchScreen.storyboard"))
                .expect("ios launch storyboard");
        let ios_runtime_header =
            fs::read_to_string(root.join("generated/ios/hello-atom/atom_runtime.h"))
                .expect("ios runtime header");
        let ios_runtime_bridge =
            fs::read_to_string(root.join("generated/ios/hello-atom/atom_runtime_app_bridge.rs"))
                .expect("ios runtime bridge");
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
        let android_manifest = fs::read_to_string(
            root.join("generated/android/hello-atom/AndroidManifest.generated.xml"),
        )
        .expect("android manifest");
        let android_runtime_bridge =
            fs::read_to_string(root.join("generated/android/hello-atom/atom_runtime_jni.rs"))
                .expect("android runtime bridge");
        let android_main =
            fs::read_to_string(root.join(
                "generated/android/hello-atom/src/main/kotlin/build/atom/hello/MainActivity.kt",
            ))
            .expect("android main");

        assert!(ios_build.contains("ios_application("));
        assert!(ios_build.contains("bundle_id = \"build.atom.hello\""));
        assert!(ios_build.contains("minimum_os_version = \"17.0\""));
        assert!(ios_build.contains("swift_interop_hint("));
        assert!(ios_build.contains("rust_static_library("));
        assert!(ios_build.contains("hdrs = [\"atom_runtime.h\"]"));
        assert!(ios_build.contains("\":atom_runtime_swift_bridge\""));
        assert!(!ios_build.contains("swift_binary("));
        assert!(ios_build.contains("resources = ["));
        assert!(ios_build.contains("\"LaunchScreen.storyboard\""));
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
        assert!(ios_runtime_header.contains("typedef struct AtomSlice"));
        assert!(ios_runtime_bridge.contains("hello_atom::atom_runtime_config()"));
        assert!(swift_app_delegate.contains("configurationForConnecting"));
        assert!(swift_main.contains("UIApplicationMain("));
        assert!(swift_main.contains("NSStringFromClass(AtomAppDelegate.self)"));
        assert!(swift_scene_delegate.contains("UIHostingController(rootView: AtomRootView())"));
        assert!(swift_scene_delegate.contains("Text(\"Hello Atom\")"));
        assert!(android_build.contains("rust_shared_library("));
        assert!(
            android_build.contains("load(\"@rules_android//rules:rules.bzl\", \"android_binary\")")
        );
        assert!(android_build.contains("android_binary("));
        assert!(android_build.contains("manifest = \"AndroidManifest.generated.xml\""));
        assert!(android_build.contains("custom_package = \"build.atom.hello\""));
        assert!(android_build.contains("srcs = [\"atom_runtime_jni.rs\"]"));
        assert!(!android_build.contains("java_binary("));
        assert!(!android_build.contains("AppEntry.kt"));
        assert!(android_manifest.contains("android:minSdkVersion=\"28\""));
        assert!(android_manifest.contains("android:targetSdkVersion=\"35\""));
        assert!(android_runtime_bridge.contains("hello_atom::atom_runtime_config()"));

        let android_app = fs::read_to_string(root.join(
            "generated/android/hello-atom/src/main/kotlin/build/atom/hello/AtomApplication.kt",
        ))
        .expect("android application");
        assert!(android_app.contains("class AtomApplication : Application()"));
        assert!(android_app.contains("System.loadLibrary(\"atom_runtime_jni\")"));
        assert!(android_app.contains("object AtomRuntimeBridge"));
        assert!(android_main.contains("class MainActivity : Activity()"));
        assert!(android_main.contains("atomApp?.sendLifecycle("));
        assert!(android_build.contains("kt_jvm_library("));
        assert!(android_build.contains("@androidsdk//:platforms/android-"));
        assert!(!android_build.contains("cc_import("));
        assert!(
            !root
                .join("generated/android/hello-atom/src/main/kotlin/build/atom/hello/AppEntry.kt")
                .exists()
        );
    }

    #[test]
    fn config_plugins_can_contribute_files_and_resources() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (mut manifest, modules) = write_fixture(&root);
        fs::create_dir_all(root.join("assets/AppIcon.icon")).expect("icon dir");
        fs::write(
            root.join("assets/AppIcon.icon/icon.json"),
            "{\"name\":\"AppIcon\"}",
        )
        .expect("icon json");
        fs::write(root.join("assets/ic_launcher.png"), "png").expect("png");
        manifest.config_plugins.push(ConfigPluginRequest {
            target_label: "//tests:fixture_plugin".to_owned(),
            id: "fixture_plugin".to_owned(),
            atom_api_level: 1,
            min_atom_version: Some("0.1.0".to_owned()),
            ios_min_deployment_target: Some("17.0".to_owned()),
            android_min_sdk: Some(28),
            config: JsonMap::new(),
        });

        let mut registry = fixture_registry();
        register_fixture_plugin(&mut registry);
        let plan = build_generation_plan(&manifest, &modules, &registry).expect("plan");
        emit_host_tree(&root, &plan).expect("host tree");

        let ios_plist =
            fs::read_to_string(root.join("generated/ios/hello-atom/Info.generated.plist"))
                .expect("ios plist");
        let ios_build =
            fs::read_to_string(root.join("generated/ios/hello-atom/BUILD.bazel")).expect("ios");
        let android_manifest = fs::read_to_string(
            root.join("generated/android/hello-atom/AndroidManifest.generated.xml"),
        )
        .expect("android manifest");
        let android_build =
            fs::read_to_string(root.join("generated/android/hello-atom/BUILD.bazel"))
                .expect("android build");

        assert!(
            root.join("generated/ios/hello-atom/resources/AppIcon.icon/icon.json")
                .exists()
        );
        assert!(
            root.join("generated/android/hello-atom/res/mipmap-xxxhdpi/ic_launcher.png")
                .exists()
        );
        assert!(ios_plist.contains("<key>CFBundleIconName</key>"));
        assert!(ios_plist.contains("<string>AppIcon</string>"));
        assert!(ios_build.contains("glob(["));
        assert!(ios_build.contains("\"resources/AppIcon.icon/**\""));
        assert!(android_manifest.contains("android:icon=\"@mipmap/ic_launcher\""));
        assert!(android_build.contains("resource_files = ["));
        assert!(android_build.contains("\"res/mipmap-xxxhdpi/ic_launcher.png\""));
    }

    #[test]
    fn automation_fixture_renders_probe_ui_for_both_platforms() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (mut manifest, modules) = write_fixture(&root);
        manifest.app.automation_fixture = true;

        let plan =
            build_generation_plan(&manifest, &modules, &ConfigPluginRegistry::new()).expect("plan");
        emit_host_tree(&root, &plan).expect("host tree");

        let ios_scene_delegate =
            fs::read_to_string(root.join("generated/ios/hello-atom/SceneDelegate.swift"))
                .expect("ios scene delegate");
        let android_manifest = fs::read_to_string(
            root.join("generated/android/hello-atom/AndroidManifest.generated.xml"),
        )
        .expect("android manifest");
        let android_main =
            fs::read_to_string(root.join(
                "generated/android/hello-atom/src/main/kotlin/build/atom/hello/MainActivity.kt",
            ))
            .expect("android main");

        assert!(ios_scene_delegate.contains("atom.fixture.primary_button"));
        assert!(ios_scene_delegate.contains("AtomAutomationRootView"));
        assert!(android_main.contains("atom.fixture.primary_button"));
        assert!(android_main.contains("AutomationClient"));
        assert!(android_manifest.contains("android.permission.INTERNET"));
        assert!(android_manifest.contains("android:usesCleartextTraffic=\"true\""));
    }

    #[test]
    fn config_plugin_directory_copies_do_not_leave_stale_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (mut manifest, modules) = write_fixture(&root);
        manifest.build.generated_root = Utf8PathBuf::from("cng-output");

        fs::create_dir_all(root.join("assets/AppIcon.icon/Assets")).expect("assets dir");
        fs::write(
            root.join("assets/AppIcon.icon/icon.json"),
            "{\"name\":\"AppIcon\"}",
        )
        .expect("icon json");
        fs::write(root.join("assets/AppIcon.icon/Assets/atom.svg"), "<svg />").expect("atom svg");
        fs::write(root.join("assets/ic_launcher.png"), "png").expect("png");
        manifest.config_plugins.push(ConfigPluginRequest {
            target_label: "//tests:fixture_plugin".to_owned(),
            id: "fixture_plugin".to_owned(),
            atom_api_level: 1,
            min_atom_version: Some("0.1.0".to_owned()),
            ios_min_deployment_target: Some("17.0".to_owned()),
            android_min_sdk: Some(28),
            config: JsonMap::new(),
        });

        let mut registry = fixture_registry();
        register_fixture_plugin(&mut registry);

        let initial_plan = build_generation_plan(&manifest, &modules, &registry).expect("plan");
        emit_host_tree(&root, &initial_plan).expect("host tree");

        let generated_svg =
            root.join("cng-output/ios/hello-atom/resources/AppIcon.icon/Assets/atom.svg");
        assert!(generated_svg.exists());

        fs::remove_file(root.join("assets/AppIcon.icon/Assets/atom.svg")).expect("remove svg");

        let second_plan = build_generation_plan(&manifest, &modules, &registry).expect("plan");
        emit_host_tree(&root, &second_plan).expect("host tree");

        assert!(!generated_svg.exists());
        assert!(
            root.join("cng-output/ios/hello-atom/resources/AppIcon.icon/Assets")
                .exists()
        );
    }

    #[test]
    fn incompatible_module_metadata_fails_before_generation() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, mut modules) = write_fixture(&root);
        modules[0].manifest.atom_api_level = 2;

        let error = build_generation_plan(&manifest, &modules, &fixture_registry())
            .expect_err("incompatible module should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ExtensionIncompatible);
    }
}
