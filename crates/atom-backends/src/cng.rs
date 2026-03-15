use std::collections::BTreeMap;

use atom_ffi::AtomResult;
use atom_manifest::{ConfigPluginRequest, NormalizedManifest};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{BackendDefinition, BackendRegistry};

#[derive(Debug, Clone, PartialEq)]
pub struct BackendPlan {
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
pub struct BackendContribution {
    pub files: Vec<ContributedFile>,
    pub metadata_entries: JsonMap,
    pub bazel_resources: Vec<String>,
    pub bazel_resource_globs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedBackend {
    pub plan: BackendPlan,
    pub metadata: JsonMap,
    pub bazel_resources: Vec<String>,
    pub bazel_resource_globs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenerationPlan {
    pub version: u16,
    pub status: String,
    pub manifest: NormalizedManifest,
    pub modules: Vec<ResolvedModule>,
    pub permissions: Vec<String>,
    pub entitlements: JsonMap,
    pub schema: SchemaPlan,
    pub contributed_files: Vec<ContributedFile>,
    pub backends: BTreeMap<String, PlannedBackend>,
    pub generated_files: Vec<Utf8PathBuf>,
    pub warnings: Vec<String>,
}

impl GenerationPlan {
    #[must_use]
    pub fn backend(&self, id: &str) -> Option<&PlannedBackend> {
        self.backends.get(id)
    }
}

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared backend contract once at the trait boundary."
)]
pub trait GenerationBackend: BackendDefinition {
    fn initialize_backend(
        &self,
        _manifest: &NormalizedManifest,
    ) -> AtomResult<Option<BackendContribution>> {
        Ok(None)
    }

    fn module_contribution(
        &self,
        _manifest: &NormalizedManifest,
        _module: &ResolvedModule,
    ) -> AtomResult<BackendContribution> {
        Ok(BackendContribution::default())
    }

    fn validate_module_compatibility(
        &self,
        _manifest: &NormalizedManifest,
        _module: &ResolvedModule,
    ) -> AtomResult<()> {
        Ok(())
    }

    fn validate_config_plugin_compatibility(
        &self,
        _manifest: &NormalizedManifest,
        _plugin: &ConfigPluginRequest,
    ) -> AtomResult<()> {
        Ok(())
    }

    fn build_backend_plan(&self, manifest: &NormalizedManifest) -> Option<BackendPlan>;

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()>;

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf>;
}

pub type GenerationBackendRegistry = BackendRegistry<Box<dyn GenerationBackend>>;
