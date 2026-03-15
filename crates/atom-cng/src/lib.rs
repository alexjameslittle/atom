mod emit;
mod module_flatbuffers;
mod rust_source;

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub use atom_backends::{
    BackendContribution, BackendPlan, ContributedFile, FileSource, GenerationBackendRegistry,
    GenerationPlan, PlannedBackend, SchemaPlan,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    AppConfig, ConfigPluginRequest, FRAMEWORK_ATOM_API_LEVEL, FRAMEWORK_VERSION, NormalizedManifest,
};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};
use flatbuffers::{FlatBufferBuilder, TableFinishedWIPOffset, WIPOffset};
use serde_json::Value;

use crate::module_flatbuffers::plan_module_flatbuffers;

pub use crate::emit::emit_host_tree;
pub use crate::emit::write_file as write_generated_file;

pub type ConfigPluginFactory = fn(&ConfigPluginRequest) -> AtomResult<Box<dyn ConfigPlugin>>;

pub trait ConfigPlugin: Send + Sync {
    fn id(&self) -> &str;

    /// # Errors
    ///
    /// Returns an error if the plugin configuration is invalid.
    fn validate(&self) -> AtomResult<()>;

    /// # Errors
    ///
    /// Returns an error if the plugin cannot produce valid backend contributions.
    fn contribute_backend(
        &self,
        backend_id: &str,
        ctx: &ConfigPluginContext<'_>,
    ) -> AtomResult<BackendContribution>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPluginContext<'a> {
    pub app: &'a AppConfig,
    pub repo_root: &'a Utf8Path,
    pub generated_root: &'a Utf8Path,
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

#[derive(Default)]
struct PendingBackendOutput {
    metadata: JsonMap,
    bazel_resources: Vec<String>,
    bazel_resource_globs: Vec<String>,
    plan: Option<BackendPlan>,
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
    registry: &GenerationBackendRegistry,
) -> AtomResult<GenerationPlan> {
    validate_extension_compatibility(manifest, modules, registry)?;

    let mut permissions = BTreeSet::new();
    let mut backends = BTreeMap::new();
    let mut contributed_files = Vec::new();
    for backend in registry.iter() {
        let Some(contribution) = backend.initialize_backend(manifest)? else {
            continue;
        };
        merge_backend_contribution(
            &mut backends,
            backend.id(),
            contribution,
            &format!("backends.{}.defaults", backend.id()),
            &mut contributed_files,
        )?;
    }
    let mut entitlements = JsonMap::new();
    let mut schema_modules = Vec::new();

    for module in modules {
        for permission in &module.manifest.permissions {
            permissions.insert(permission.clone());
        }
        for backend in registry.iter() {
            if !backends.contains_key(backend.id()) {
                continue;
            }
            merge_backend_contribution(
                &mut backends,
                backend.id(),
                backend.module_contribution(manifest, module)?,
                &format!("modules.{}.{}", module.manifest.id, backend.id()),
                &mut contributed_files,
            )?;
        }
        deep_merge_map(
            &mut entitlements,
            &module.manifest.entitlements,
            &format!("entitlements.{}", module.manifest.id),
        )?;

        let flatbuffer_package = plan_module_flatbuffers(&manifest.repo_root, module)?;
        if let Some(schema) = &flatbuffer_package.generated_schema {
            schema_modules.push(schema.path.clone());
            contributed_files.push(ContributedFile {
                source: FileSource::Content(schema.contents.clone()),
                output: schema.path.clone(),
            });
        } else {
            schema_modules.extend(module.manifest.schema_files.iter().cloned());
        }
        contributed_files.push(ContributedFile {
            source: FileSource::Content(flatbuffer_package.build_contents()),
            output: flatbuffer_package.build_file.clone(),
        });
        contributed_files.push(ContributedFile {
            source: FileSource::Content(flatbuffer_package.rust_wrapper_contents()),
            output: flatbuffer_package.rust_wrapper.clone(),
        });
    }

    let plugin_ctx = ConfigPluginContext {
        app: &manifest.app,
        repo_root: &manifest.repo_root,
        generated_root: &manifest.build.generated_root,
    };
    for entry in &manifest.config_plugins {
        let plugin = config_plugins.instantiate(entry)?;
        plugin.validate()?;

        for backend_id in backends.keys().cloned().collect::<Vec<_>>() {
            merge_backend_contribution(
                &mut backends,
                &backend_id,
                plugin.contribute_backend(&backend_id, &plugin_ctx)?,
                &format!("config_plugins.{}.{}", entry.id, backend_id),
                &mut contributed_files,
            )?;
        }
    }

    let schema = SchemaPlan {
        aggregate: Utf8PathBuf::new(),
        modules: schema_modules,
    };

    for backend in registry.iter() {
        if !backends.contains_key(backend.id()) {
            continue;
        }
        let Some(backend_plan) = backend.build_backend_plan(manifest) else {
            continue;
        };
        backends.entry(backend.id().to_owned()).or_default().plan = Some(backend_plan);
    }

    let mut generated_files: Vec<Utf8PathBuf> = contributed_files
        .iter()
        .map(|file| file.output.clone())
        .collect();
    let backends: BTreeMap<String, PlannedBackend> = backends
        .into_iter()
        .filter_map(|(id, pending)| {
            pending.plan.map(|plan| {
                generated_files.extend(plan.files.iter().cloned());
                (
                    id,
                    PlannedBackend {
                        plan,
                        metadata: pending.metadata,
                        bazel_resources: pending.bazel_resources,
                        bazel_resource_globs: pending.bazel_resource_globs,
                    },
                )
            })
        })
        .collect();

    Ok(GenerationPlan {
        version: 1,
        status: "dry-run".to_owned(),
        manifest: manifest.clone(),
        modules: modules.to_vec(),
        permissions: permissions.into_iter().collect(),
        entitlements,
        schema,
        contributed_files,
        backends,
        generated_files,
        warnings: Vec::new(),
    })
}

#[must_use]
pub fn render_prebuild_plan(plan: &GenerationPlan) -> Vec<u8> {
    let mut builder = FlatBufferBuilder::new();

    let status = builder.create_string(&plan.status);
    let app_name = builder.create_string(&plan.manifest.app.name);
    let app_slug = builder.create_string(&plan.manifest.app.slug);
    let app_entry_target = builder.create_string(&plan.manifest.app.entry_crate_label);
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

    let mut backend_offsets = Vec::with_capacity(plan.backends.len());
    for (backend_id, backend) in &plan.backends {
        let id = builder.create_string(backend_id);
        let generated_root = builder.create_string(backend.plan.generated_root.as_str());
        let target = builder.create_string(&backend.plan.target);
        backend_offsets.push(create_prebuild_backend(
            &mut builder,
            id,
            generated_root,
            target,
        ));
    }
    let backends = builder.create_vector(backend_offsets.as_slice());

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
        builder.push_slot_always::<WIPOffset<_>>(12, backends);
        builder.push_slot_always::<WIPOffset<_>>(14, schema);
        builder.push_slot_always::<WIPOffset<_>>(16, generated_files);
        builder.push_slot_always::<WIPOffset<_>>(18, warnings);
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

fn create_prebuild_backend<'a>(
    builder: &mut FlatBufferBuilder<'a>,
    id: WIPOffset<&'a str>,
    generated_root: WIPOffset<&'a str>,
    target: WIPOffset<&'a str>,
) -> WIPOffset<TableFinishedWIPOffset> {
    let table = builder.start_table();
    builder.push_slot_always::<WIPOffset<_>>(4, id);
    builder.push_slot_always::<WIPOffset<_>>(6, generated_root);
    builder.push_slot_always::<WIPOffset<_>>(8, target);
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

fn validate_extension_compatibility(
    manifest: &NormalizedManifest,
    modules: &[ResolvedModule],
    registry: &GenerationBackendRegistry,
) -> AtomResult<()> {
    for module in modules {
        validate_extension(
            module.manifest.target_label.as_str(),
            module.manifest.atom_api_level,
            module.manifest.min_atom_version.as_deref(),
        )?;
        for backend in registry.iter() {
            backend.validate_module_compatibility(manifest, module)?;
        }
    }
    for plugin in &manifest.config_plugins {
        validate_extension(
            plugin.target_label.as_str(),
            plugin.atom_api_level,
            plugin.min_atom_version.as_deref(),
        )?;
        for backend in registry.iter() {
            backend.validate_config_plugin_compatibility(manifest, plugin)?;
        }
    }
    Ok(())
}

fn validate_extension(
    target_label: &str,
    atom_api_level: u32,
    min_atom_version: Option<&str>,
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

    Ok(())
}

fn extension_compatibility_error(target_label: &str, field: &str, message: String) -> AtomError {
    AtomError::with_path(
        AtomErrorCode::ExtensionIncompatible,
        message,
        format!("{target_label}.{field}"),
    )
}

fn compare_semver(current: &str, required: &str) -> AtomResult<Ordering> {
    let current = parse_semver(current)?;
    let required = parse_semver(required)?;
    Ok(current.cmp(&required))
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

fn merge_backend_contribution(
    backends: &mut BTreeMap<String, PendingBackendOutput>,
    backend_id: &str,
    contribution: BackendContribution,
    metadata_context: &str,
    contributed_files: &mut Vec<ContributedFile>,
) -> AtomResult<()> {
    let backend = backends.entry(backend_id.to_owned()).or_default();
    deep_merge_map(
        &mut backend.metadata,
        &contribution.metadata_entries,
        metadata_context,
    )?;
    contributed_files.extend(contribution.files);
    backend.bazel_resources.extend(contribution.bazel_resources);
    backend
        .bazel_resource_globs
        .extend(contribution.bazel_resource_globs);
    Ok(())
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

#[cfg(test)]
fn object_from_value(value: Value) -> JsonMap {
    match value {
        Value::Object(map) => map,
        _ => JsonMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_backends::BackendDefinition;
    use atom_backends::{
        BackendContribution, BackendPlan, ContributedFile, FileSource, GenerationBackend,
        GenerationBackendRegistry, GenerationPlan,
    };
    use atom_manifest::{
        NormalizedManifest,
        testing::{fixture_config_plugin_request, fixture_manifest, fixture_module_request},
    };
    use atom_modules::{ResolvedModule, testing::fixture_resolved_module};
    use camino::{Utf8Path, Utf8PathBuf};
    use serde_json::{Value, json};
    use tempfile::tempdir;

    use super::{
        ConfigPlugin, ConfigPluginContext, ConfigPluginRegistry, build_generation_plan,
        emit_host_tree, object_from_value, render_prebuild_plan,
    };

    struct FixturePlugin;

    impl ConfigPlugin for FixturePlugin {
        fn id(&self) -> &str {
            "fixture_plugin"
        }

        fn validate(&self) -> atom_ffi::AtomResult<()> {
            Ok(())
        }

        fn contribute_backend(
            &self,
            backend_id: &str,
            ctx: &ConfigPluginContext<'_>,
        ) -> atom_ffi::AtomResult<BackendContribution> {
            match backend_id {
                "alpha" => Ok(BackendContribution {
                    files: vec![ContributedFile {
                        source: FileSource::Copy(Utf8PathBuf::from("assets/alpha")),
                        output: ctx
                            .generated_root
                            .join("alpha")
                            .join("resources")
                            .join("alpha"),
                    }],
                    metadata_entries: object_from_value(json!({
                        "plugin_marker": "alpha"
                    })),
                    bazel_resources: vec!["resources/alpha/logo.txt".to_owned()],
                    bazel_resource_globs: vec!["resources/alpha/**".to_owned()],
                }),
                "beta" => Ok(BackendContribution {
                    files: vec![ContributedFile {
                        source: FileSource::Copy(Utf8PathBuf::from("assets/beta.txt")),
                        output: ctx
                            .generated_root
                            .join("beta")
                            .join("resources")
                            .join("beta.txt"),
                    }],
                    metadata_entries: object_from_value(json!({
                        "plugin_marker": "beta"
                    })),
                    bazel_resources: vec!["resources/beta.txt".to_owned()],
                    bazel_resource_globs: Vec::new(),
                }),
                _ => Ok(BackendContribution::default()),
            }
        }
    }

    fn register_fixture_plugin(registry: &mut ConfigPluginRegistry) {
        registry.register("fixture_plugin", |_entry| Ok(Box::new(FixturePlugin)));
    }

    struct FixtureBackend {
        id: &'static str,
    }

    impl BackendDefinition for FixtureBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platform(&self) -> &'static str {
            self.id
        }
    }

    impl GenerationBackend for FixtureBackend {
        fn initialize_backend(
            &self,
            _manifest: &NormalizedManifest,
        ) -> atom_ffi::AtomResult<Option<BackendContribution>> {
            Ok(Some(BackendContribution::default()))
        }

        fn build_backend_plan(&self, manifest: &NormalizedManifest) -> Option<BackendPlan> {
            let generated_root = manifest.build.generated_root.join(self.id);
            Some(BackendPlan {
                generated_root: generated_root.clone(),
                target: format!("//{}:app", generated_root.as_str()),
                files: vec![generated_root.join("FIXTURE.txt")],
            })
        }

        fn emit_host_tree(
            &self,
            repo_root: &Utf8Path,
            plan: &GenerationPlan,
        ) -> atom_ffi::AtomResult<()> {
            crate::emit::write_file(
                &repo_root.join(
                    plan.backend(self.id)
                        .expect("fixture backend should populate backend plan")
                        .plan
                        .generated_root
                        .join("FIXTURE.txt"),
                ),
                self.id,
            )
        }

        fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
            plan.backend(self.id)
                .map(|backend| backend.plan.generated_root.clone())
        }
    }

    fn fixture_manifest_and_modules(
        root: &Utf8PathBuf,
    ) -> (NormalizedManifest, Vec<ResolvedModule>) {
        fs::create_dir_all(root.join("modules/fixture/src")).expect("module dir");
        fs::write(
            root.join("modules/fixture/src/lib.rs"),
            r#"
#[atom_macros::atom_record]
pub struct DeviceInfo {
    pub model: String,
    pub os: String,
}
"#,
        )
        .expect("source");

        let mut manifest = fixture_manifest(root);
        manifest.modules = vec![fixture_module_request("//modules/fixture:fixture")];

        let modules = vec![fixture_resolved_module(root)];
        (manifest, modules)
    }

    fn fixture_registry() -> ConfigPluginRegistry {
        ConfigPluginRegistry::default()
    }

    fn generation_registry(ids: &[&'static str]) -> GenerationBackendRegistry {
        let mut registry = GenerationBackendRegistry::new();
        for id in ids {
            registry
                .register(Box::new(FixtureBackend { id }))
                .expect("fixture backend should register");
        }
        registry
    }

    #[test]
    fn plan_contains_schema_and_registered_backend_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = fixture_manifest_and_modules(&root);
        let registry = generation_registry(&["fixture"]);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry(), &registry)
            .expect("plan");

        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/flatbuffers/fixture_module/BUILD.bazel"
        )));
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/flatbuffers/fixture_module/lib.rs"
        )));
        assert!(plan.generated_files.contains(&Utf8PathBuf::from(
            "generated/flatbuffers/fixture_module/fixture_module.fbs"
        )));
        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/fixture/FIXTURE.txt"))
        );
        assert_eq!(plan.schema.aggregate, Utf8PathBuf::new());
        assert_eq!(
            plan.schema.modules,
            vec![Utf8PathBuf::from(
                "generated/flatbuffers/fixture_module/fixture_module.fbs"
            )]
        );
        assert!(!render_prebuild_plan(&plan).is_empty());
    }

    #[test]
    fn emit_host_tree_writes_schema_and_backend_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = fixture_manifest_and_modules(&root);
        let registry = generation_registry(&["fixture"]);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry(), &registry)
            .expect("plan");

        let roots = emit_host_tree(&root, &plan, &registry).expect("host tree");

        assert!(
            root.join("generated/flatbuffers/fixture_module/fixture_module.fbs")
                .exists()
        );
        assert!(
            root.join("generated/flatbuffers/fixture_module/BUILD.bazel")
                .exists()
        );
        assert!(
            root.join("generated/flatbuffers/fixture_module/lib.rs")
                .exists()
        );
        assert!(root.join("generated/fixture/FIXTURE.txt").exists());
        assert_eq!(roots, vec![Utf8PathBuf::from("generated/fixture")]);
    }

    #[test]
    fn cng_uses_registered_backends_for_planning_and_emission() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = fixture_manifest_and_modules(&root);
        let registry = generation_registry(&["fixture"]);

        let plan = build_generation_plan(&manifest, &modules, &fixture_registry(), &registry)
            .expect("plan");
        assert_eq!(
            plan.backend("fixture")
                .expect("fixture backend should produce fixture plan")
                .plan
                .generated_root,
            Utf8PathBuf::from("generated/fixture")
        );
        assert!(plan.backend("secondary").is_none());
        assert!(
            plan.generated_files
                .contains(&Utf8PathBuf::from("generated/fixture/FIXTURE.txt"))
        );

        let roots = emit_host_tree(&root, &plan, &registry).expect("host tree");
        assert_eq!(roots, vec![Utf8PathBuf::from("generated/fixture")]);
        assert_eq!(
            fs::read_to_string(root.join("generated/fixture/FIXTURE.txt")).expect("fixture"),
            "fixture"
        );
    }

    #[test]
    fn config_plugins_contribute_backend_owned_files_and_metadata() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (mut manifest, modules) = fixture_manifest_and_modules(&root);
        fs::create_dir_all(root.join("assets/alpha")).expect("alpha assets");
        fs::write(root.join("assets/alpha/logo.txt"), "alpha").expect("alpha asset");
        fs::write(root.join("assets/beta.txt"), "beta").expect("beta asset");
        manifest.config_plugins.push(fixture_config_plugin_request(
            "fixture_plugin",
            "//tests:fixture_plugin",
        ));

        let mut registry = fixture_registry();
        register_fixture_plugin(&mut registry);
        let generation_registry = generation_registry(&["alpha", "beta"]);
        let plan = build_generation_plan(&manifest, &modules, &registry, &generation_registry)
            .expect("plan");
        emit_host_tree(&root, &plan, &generation_registry).expect("host tree");

        assert!(
            root.join("generated/alpha/resources/alpha/logo.txt")
                .exists()
        );
        assert!(root.join("generated/beta/resources/beta.txt").exists());
        assert_eq!(
            plan.backend("alpha")
                .and_then(|backend| backend.metadata.get("plugin_marker"))
                .cloned(),
            Some(Value::String("alpha".to_owned()))
        );
        assert_eq!(
            plan.backend("beta")
                .and_then(|backend| backend.metadata.get("plugin_marker"))
                .cloned(),
            Some(Value::String("beta".to_owned()))
        );
        assert_eq!(
            plan.backend("alpha")
                .expect("alpha backend")
                .bazel_resource_globs,
            vec!["resources/alpha/**".to_owned()]
        );
        assert_eq!(
            plan.backend("beta").expect("beta backend").bazel_resources,
            vec!["resources/beta.txt".to_owned()]
        );
    }

    #[test]
    fn config_plugin_directory_copies_do_not_leave_stale_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (mut manifest, modules) = fixture_manifest_and_modules(&root);
        manifest.build.generated_root = Utf8PathBuf::from("cng-output");

        fs::create_dir_all(root.join("assets/alpha/subdir")).expect("assets dir");
        fs::write(root.join("assets/alpha/logo.txt"), "alpha").expect("alpha file");
        fs::write(root.join("assets/alpha/subdir/detail.txt"), "detail").expect("detail file");
        fs::write(root.join("assets/beta.txt"), "beta").expect("beta");
        manifest.config_plugins.push(fixture_config_plugin_request(
            "fixture_plugin",
            "//tests:fixture_plugin",
        ));

        let mut registry = fixture_registry();
        register_fixture_plugin(&mut registry);

        let generation_registry = generation_registry(&["alpha"]);
        let initial_plan =
            build_generation_plan(&manifest, &modules, &registry, &generation_registry)
                .expect("plan");
        emit_host_tree(&root, &initial_plan, &generation_registry).expect("host tree");

        let generated_detail = root.join("cng-output/alpha/resources/alpha/subdir/detail.txt");
        assert!(generated_detail.exists());

        fs::remove_file(root.join("assets/alpha/subdir/detail.txt")).expect("remove detail");

        let second_plan =
            build_generation_plan(&manifest, &modules, &registry, &generation_registry)
                .expect("plan");
        emit_host_tree(&root, &second_plan, &generation_registry).expect("host tree");

        assert!(!generated_detail.exists());
        assert!(
            root.join("cng-output/alpha/resources/alpha/subdir")
                .exists()
        );
    }

    #[test]
    fn stale_flatbuffer_module_packages_are_removed() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, modules) = fixture_manifest_and_modules(&root);
        fs::create_dir_all(root.join("modules/fixture_two/src")).expect("module dir");
        fs::write(
            root.join("modules/fixture_two/src/lib.rs"),
            r#"
#[atom_macros::atom_record]
pub struct SecondaryInfo {
    pub value: String,
}
"#,
        )
        .expect("source");
        let mut second_module = fixture_resolved_module(&root);
        second_module.request.target_label = "//modules/fixture_two:fixture_two".to_owned();
        second_module.metadata_path = root.join("fixture_two.atom.module.json");
        second_module.manifest.target_label = "//modules/fixture_two:fixture_two".to_owned();
        second_module.manifest.id = "fixture_two".to_owned();
        second_module.manifest.crate_root =
            Some(Utf8PathBuf::from("modules/fixture_two/src/lib.rs"));
        let registry = generation_registry(&["fixture"]);

        let initial_plan = build_generation_plan(
            &manifest,
            &[modules[0].clone(), second_module.clone()],
            &fixture_registry(),
            &registry,
        )
        .expect("plan");
        emit_host_tree(&root, &initial_plan, &registry).expect("host tree");
        assert!(root.join("generated/flatbuffers/fixture_module").exists());
        assert!(root.join("generated/flatbuffers/fixture_two").exists());

        let empty_plan = build_generation_plan(&manifest, &modules, &fixture_registry(), &registry)
            .expect("plan");
        emit_host_tree(&root, &empty_plan, &registry).expect("host tree");

        assert!(root.join("generated/flatbuffers/fixture_module").exists());
        assert!(!root.join("generated/flatbuffers/fixture_two").exists());
    }

    #[test]
    fn incompatible_module_metadata_fails_before_generation() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let (manifest, mut modules) = fixture_manifest_and_modules(&root);
        modules[0].manifest.atom_api_level = 2;

        let generation_registry = generation_registry(&["fixture"]);
        let error = build_generation_plan(
            &manifest,
            &modules,
            &fixture_registry(),
            &generation_registry,
        )
        .expect_err("incompatible module should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ExtensionIncompatible);
    }
}
