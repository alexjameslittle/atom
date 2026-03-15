use camino::{Utf8Path, Utf8PathBuf};

use crate::{JsonMap, ModuleKind, ModuleManifest, ResolvedModule};
use atom_manifest::ModuleRequest;

#[must_use]
pub fn fixture_resolved_module(repo_root: &Utf8Path) -> ResolvedModule {
    ResolvedModule {
        request: ModuleRequest {
            target_label: "//modules/fixture:fixture".to_owned(),
        },
        metadata_path: repo_root.join("fixture.atom.module.json"),
        manifest: ModuleManifest {
            kind: ModuleKind::Rust,
            target_label: "//modules/fixture:fixture".to_owned(),
            id: "fixture_module".to_owned(),
            atom_api_level: 1,
            min_atom_version: Some("0.1.0".to_owned()),
            ios_min_deployment_target: None,
            android_min_sdk: None,
            crate_root: Some(Utf8PathBuf::from("modules/fixture/src/lib.rs")),
            generated_root: Utf8PathBuf::from("generated"),
            depends_on: Vec::new(),
            schema_files: Vec::new(),
            methods: Vec::new(),
            permissions: Vec::new(),
            plist: JsonMap::new(),
            android_manifest: JsonMap::new(),
            entitlements: JsonMap::new(),
            generated_sources: Vec::new(),
            init_priority: 0,
            ios_srcs: Vec::new(),
            android_srcs: Vec::new(),
        },
        resolution_index: 0,
        layer: 0,
        init_order: 0,
    }
}

#[must_use]
pub fn fixture_schema_module(repo_root: &Utf8Path, schema_path: &str) -> ResolvedModule {
    let mut module = fixture_resolved_module(repo_root);
    module.manifest.kind = ModuleKind::Native;
    "schema_module".clone_into(&mut module.manifest.id);
    "//modules/schema:schema".clone_into(&mut module.request.target_label);
    module.metadata_path = repo_root.join("schema.atom.module.json");
    "//modules/schema:schema".clone_into(&mut module.manifest.target_label);
    module.manifest.crate_root = None;
    module.manifest.schema_files = vec![Utf8PathBuf::from(schema_path)];
    module
}
