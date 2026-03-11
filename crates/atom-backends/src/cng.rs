use atom_ffi::AtomResult;
use atom_manifest::{AndroidConfig, AppConfig, BuildConfig, IosConfig, NormalizedManifest};
use atom_modules::{JsonMap, ResolvedModule};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{BackendDefinition, BackendRegistry};

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

#[expect(
    clippy::missing_errors_doc,
    reason = "Trait methods document the shared backend contract once at the trait boundary."
)]
pub trait GenerationBackend: BackendDefinition {
    fn build_platform_plan(&self, manifest: &NormalizedManifest) -> Option<PlatformPlan>;

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()>;

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf>;
}

pub type GenerationBackendRegistry = BackendRegistry<Box<dyn GenerationBackend>>;
