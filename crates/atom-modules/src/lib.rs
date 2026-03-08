mod graph;
mod loader;

use std::collections::BTreeSet;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    MODULE_METADATA_SUFFIX, ModuleRequest, build_metadata_output, metadata_target,
};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::graph::resolve_loaded_modules;
use crate::loader::load_module_manifest_from_path;

pub type JsonMap = Map<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodSpec {
    pub name: String,
    pub request_table: String,
    pub response_table: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleKind {
    Rust,
    Native,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleManifest {
    pub kind: ModuleKind,
    pub target_label: String,
    pub id: String,
    pub depends_on: Vec<String>,
    pub schema_files: Vec<Utf8PathBuf>,
    pub methods: Vec<MethodSpec>,
    pub permissions: Vec<String>,
    pub plist: JsonMap,
    pub android_manifest: JsonMap,
    pub entitlements: JsonMap,
    pub generated_sources: Vec<String>,
    pub init_priority: i32,
    pub ios_srcs: Vec<Utf8PathBuf>,
    pub android_srcs: Vec<Utf8PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedModule {
    pub request: ModuleRequest,
    pub metadata_path: Utf8PathBuf,
    pub manifest: ModuleManifest,
    pub resolution_index: usize,
    pub layer: usize,
    pub init_order: usize,
}

pub trait AtomModule {
    fn manifest() -> ModuleManifest;
    fn exports(exports: &mut ModuleExports);
}

#[derive(Debug, Default)]
pub struct ModuleExports {
    pub methods: Vec<MethodSpec>,
}

impl ModuleExports {
    pub fn export(&mut self, method: MethodSpec) {
        self.methods.push(method);
    }
}

/// # Errors
///
/// Returns an error if any module target is duplicated, metadata cannot be
/// loaded, or the dependency graph contains a cycle.
pub fn resolve_modules(
    repo_root: &Utf8Path,
    requests: &[ModuleRequest],
) -> AtomResult<Vec<ResolvedModule>> {
    let mut loaded = Vec::with_capacity(requests.len());
    let mut seen_targets = BTreeSet::new();

    for request in requests {
        if !seen_targets.insert(request.target_label.clone()) {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleDuplicateId,
                format!("duplicate module target label: {}", request.target_label),
                request.target_label.as_str(),
            ));
        }

        let metadata_label = metadata_target(&request.target_label, MODULE_METADATA_SUFFIX)?;
        let metadata_path = build_metadata_output(repo_root, &metadata_label)?;
        let manifest =
            load_module_manifest_from_path(repo_root, &request.target_label, &metadata_path)?;
        loaded.push((request.clone(), metadata_path, manifest));
    }

    resolve_loaded_modules(&loaded)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_manifest::ModuleRequest;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use crate::graph::resolve_loaded_modules;
    use crate::loader::load_module_manifest_from_path;

    use super::{JsonMap, MethodSpec, ModuleKind, ModuleManifest};

    fn write_metadata(root: &Utf8PathBuf, output_name: &str, contents: &str) -> Utf8PathBuf {
        let metadata_path = root.join(output_name);
        fs::write(&metadata_path, contents).expect("metadata should write");
        metadata_path
    }

    #[test]
    fn loads_rust_module_metadata() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("modules/device_info/schema")).expect("schema dir");
        fs::create_dir_all(root.join("modules/device_info/src")).expect("src dir");
        fs::write(
            root.join("modules/device_info/schema/device_info.fbs"),
            "namespace atom.device_info;\n",
        )
        .expect("schema");
        fs::write(
            root.join("modules/device_info/src/device_info.swift"),
            "final class DeviceInfoModule {}\n",
        )
        .expect("swift");

        let metadata_path = write_metadata(
            &root,
            "device_info.atom.module.json",
            r#"{
  "kind": "atom_module",
  "target_label": "//modules/device_info:device_info",
  "id": "device_info",
  "depends_on": [],
  "schema_files": ["modules/device_info/schema/device_info.fbs"],
  "methods": [
    {
      "name": "get",
      "request_table": "atom.device_info.GetDeviceInfoRequest",
      "response_table": "atom.device_info.GetDeviceInfoResponse"
    }
  ],
  "permissions": [],
  "plist": {},
  "android_manifest": {},
  "entitlements": {},
  "generated_sources": [],
  "init_priority": 0,
  "ios_srcs": ["modules/device_info/src/device_info.swift"],
  "android_srcs": []
}"#,
        );

        let manifest = load_module_manifest_from_path(
            &root,
            "//modules/device_info:device_info",
            &metadata_path,
        )
        .expect("metadata should load");

        assert_eq!(manifest.kind, ModuleKind::Rust);
        assert_eq!(manifest.id, "device_info");
        assert_eq!(
            manifest.schema_files,
            vec![Utf8PathBuf::from(
                "modules/device_info/schema/device_info.fbs"
            )]
        );
        assert_eq!(
            manifest.ios_srcs,
            vec![Utf8PathBuf::from(
                "modules/device_info/src/device_info.swift"
            )]
        );
    }

    #[test]
    fn rejects_missing_schema_files() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let metadata_path = write_metadata(
            &root,
            "device_info.atom.module.json",
            r#"{
  "kind": "atom_native_module",
  "target_label": "//modules/device_info:device_info",
  "id": "device_info",
  "depends_on": [],
  "schema_files": [],
  "methods": [],
  "permissions": [],
  "plist": {},
  "android_manifest": {},
  "entitlements": {},
  "generated_sources": [],
  "init_priority": 0,
  "ios_srcs": [],
  "android_srcs": []
}"#,
        );

        let error = load_module_manifest_from_path(
            &root,
            "//modules/device_info:device_info",
            &metadata_path,
        )
        .expect_err("schema files should be required");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ModuleManifestInvalid);
    }

    #[test]
    fn resolves_dependency_layers_by_target_label() {
        let resolved = resolve_loaded_modules(&[
            (
                ModuleRequest {
                    target_label: "//modules/a:a".to_owned(),
                },
                Utf8PathBuf::from("a.atom.module.json"),
                ModuleManifest {
                    kind: ModuleKind::Rust,
                    target_label: "//modules/a:a".to_owned(),
                    id: "a".to_owned(),
                    depends_on: Vec::new(),
                    schema_files: vec![Utf8PathBuf::from("modules/a/schema/a.fbs")],
                    methods: vec![MethodSpec {
                        name: "a".to_owned(),
                        request_table: "atom.a.Request".to_owned(),
                        response_table: "atom.a.Response".to_owned(),
                    }],
                    permissions: Vec::new(),
                    plist: JsonMap::new(),
                    android_manifest: JsonMap::new(),
                    entitlements: JsonMap::new(),
                    generated_sources: Vec::new(),
                    init_priority: 0,
                    ios_srcs: Vec::new(),
                    android_srcs: Vec::new(),
                },
            ),
            (
                ModuleRequest {
                    target_label: "//modules/b:b".to_owned(),
                },
                Utf8PathBuf::from("b.atom.module.json"),
                ModuleManifest {
                    kind: ModuleKind::Native,
                    target_label: "//modules/b:b".to_owned(),
                    id: "b".to_owned(),
                    depends_on: vec!["//modules/a:a".to_owned()],
                    schema_files: vec![Utf8PathBuf::from("modules/b/schema/b.fbs")],
                    methods: vec![MethodSpec {
                        name: "b".to_owned(),
                        request_table: "atom.b.Request".to_owned(),
                        response_table: "atom.b.Response".to_owned(),
                    }],
                    permissions: Vec::new(),
                    plist: JsonMap::new(),
                    android_manifest: JsonMap::new(),
                    entitlements: JsonMap::new(),
                    generated_sources: Vec::new(),
                    init_priority: 5,
                    ios_srcs: vec![Utf8PathBuf::from("modules/b/ios/Module.swift")],
                    android_srcs: Vec::new(),
                },
            ),
        ])
        .expect("modules should resolve");

        assert_eq!(resolved[0].manifest.id, "a");
        assert_eq!(resolved[0].layer, 0);
        assert_eq!(resolved[1].manifest.id, "b");
        assert_eq!(resolved[1].layer, 1);
    }
}
