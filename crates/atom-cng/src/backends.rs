use atom_backends::{BackendDefinition, BackendRegistry};
use atom_ffi::AtomResult;
use atom_manifest::NormalizedManifest;
use camino::{Utf8Path, Utf8PathBuf};

use crate::android::{build_android_plan, emit_android_host_tree};
use crate::ios::{build_ios_plan, emit_ios_host_tree};
use crate::{GenerationPlan, PlatformPlan};

pub trait GenerationBackend: BackendDefinition {
    fn build_platform_plan(&self, manifest: &NormalizedManifest) -> Option<PlatformPlan>;

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()>;

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf>;
}

pub type GenerationBackendRegistry = BackendRegistry<Box<dyn GenerationBackend>>;

#[must_use]
pub fn first_party_generation_backend_registry() -> GenerationBackendRegistry {
    let mut registry = GenerationBackendRegistry::new();
    registry
        .register(Box::new(IosGenerationBackend))
        .expect("first-party iOS backend id should be unique");
    registry
        .register(Box::new(AndroidGenerationBackend))
        .expect("first-party Android backend id should be unique");
    registry
}

struct IosGenerationBackend;

impl BackendDefinition for IosGenerationBackend {
    fn id(&self) -> &'static str {
        "ios"
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

struct AndroidGenerationBackend;

impl BackendDefinition for AndroidGenerationBackend {
    fn id(&self) -> &'static str {
        "android"
    }

    fn platform(&self) -> &'static str {
        "android"
    }
}

impl GenerationBackend for AndroidGenerationBackend {
    fn build_platform_plan(&self, manifest: &NormalizedManifest) -> Option<PlatformPlan> {
        manifest
            .android
            .enabled
            .then(|| build_android_plan(&manifest.app, &manifest.build, &manifest.android))
    }

    fn emit_host_tree(&self, repo_root: &Utf8Path, plan: &GenerationPlan) -> AtomResult<()> {
        emit_android_host_tree(repo_root, plan)
    }

    fn generated_root(&self, plan: &GenerationPlan) -> Option<Utf8PathBuf> {
        plan.android
            .as_ref()
            .map(|android| android.generated_root.clone())
    }
}
