use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::{
    MODULE_METADATA_SUFFIX, ModuleRequest, build_metadata_output, metadata_target,
};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::{Map, Value};

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModuleManifest {
    kind: String,
    target_label: String,
    id: String,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    schema_files: Vec<String>,
    #[serde(default)]
    methods: Vec<MethodSpec>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    plist: JsonMap,
    #[serde(default)]
    android_manifest: JsonMap,
    #[serde(default)]
    entitlements: JsonMap,
    #[serde(default)]
    generated_sources: Vec<String>,
    #[serde(default)]
    init_priority: i32,
    #[serde(default)]
    ios_srcs: Vec<String>,
    #[serde(default)]
    android_srcs: Vec<String>,
}

type LoadedModule = (ModuleRequest, Utf8PathBuf, ModuleManifest);

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

fn resolve_loaded_modules(loaded: &[LoadedModule]) -> AtomResult<Vec<ResolvedModule>> {
    let mut by_id = HashMap::new();
    let mut by_target = HashMap::new();

    for (index, (_, _, manifest)) in loaded.iter().enumerate() {
        if by_id.insert(manifest.id.clone(), index).is_some() {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleDuplicateId,
                format!("duplicate module identifier: {}", manifest.id),
                manifest.id.as_str(),
            ));
        }
        by_target.insert(manifest.target_label.clone(), index);
    }

    let mut indegree = vec![0usize; loaded.len()];
    let mut dependents = vec![Vec::new(); loaded.len()];
    let mut layers = vec![0usize; loaded.len()];

    for (index, (_, _, manifest)) in loaded.iter().enumerate() {
        for dependency in &manifest.depends_on {
            let Some(&dependency_index) = by_target.get(dependency) else {
                return Err(AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    format!("unknown dependency target: {dependency}"),
                    format!("modules.{}.depends_on", manifest.id),
                ));
            };
            indegree[index] += 1;
            dependents[dependency_index].push(index);
        }
    }

    let mut ready = VecDeque::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push_back(index);
        }
    }

    let mut resolved_indices = Vec::with_capacity(loaded.len());
    while let Some(index) = ready.pop_front() {
        resolved_indices.push(index);

        let mut children = dependents[index].clone();
        children.sort_unstable();
        for child in children {
            layers[child] = layers[child].max(layers[index] + 1);
            indegree[child] -= 1;
            if indegree[child] == 0 {
                insert_ready(&mut ready, child);
            }
        }
    }

    if resolved_indices.len() != loaded.len() {
        return Err(AtomError::new(
            AtomErrorCode::ModuleDependencyCycle,
            "module dependency cycle detected",
        ));
    }

    let mut init_order = resolved_indices.clone();
    init_order.sort_by_key(|index| {
        (
            layers[*index],
            Reverse(loaded[*index].2.init_priority),
            *index,
        )
    });

    let mut init_positions = HashMap::new();
    for (position, index) in init_order.into_iter().enumerate() {
        init_positions.insert(index, position);
    }

    Ok(resolved_indices
        .into_iter()
        .enumerate()
        .map(|(resolution_index, index)| {
            let (request, metadata_path, manifest) = loaded[index].clone();
            ResolvedModule {
                request,
                metadata_path,
                manifest,
                resolution_index,
                layer: layers[index],
                init_order: init_positions[&index],
            }
        })
        .collect())
}

fn insert_ready(ready: &mut VecDeque<usize>, index: usize) {
    let mut items: Vec<_> = ready.drain(..).collect();
    items.push(index);
    items.sort_unstable();
    ready.extend(items);
}

fn load_module_manifest_from_path(
    repo_root: &Utf8Path,
    requested_target: &str,
    metadata_path: &Utf8Path,
) -> AtomResult<ModuleManifest> {
    if !metadata_path.exists() {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            "atom module metadata file could not be found",
            metadata_path.as_str(),
        ));
    }

    let raw = fs::read_to_string(metadata_path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            format!("failed to read module metadata: {error}"),
            metadata_path.as_str(),
        )
    })?;

    let parsed: RawModuleManifest = serde_json::from_str(&raw).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            format!("failed to parse module metadata JSON: {error}"),
            metadata_path.as_str(),
        )
    })?;

    let kind = match parsed.kind.as_str() {
        "atom_module" => ModuleKind::Rust,
        "atom_native_module" => ModuleKind::Native,
        _ => {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("unsupported module metadata kind: {}", parsed.kind),
                metadata_path.as_str(),
            ));
        }
    };

    if parsed.target_label != requested_target {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            format!(
                "module metadata target {} does not match requested target {}",
                parsed.target_label, requested_target
            ),
            metadata_path.as_str(),
        ));
    }

    let id = parsed.id.trim();
    if id.is_empty() {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            "module identifier must be non-empty",
            metadata_path.as_str(),
        ));
    }

    let depends_on = validate_labels(parsed.depends_on, "depends_on", metadata_path)?;
    let schema_files = validate_repo_relative_paths(
        repo_root,
        parsed.schema_files,
        "schema_files",
        metadata_path,
        true,
    )?;
    let ios_srcs =
        validate_repo_relative_paths(repo_root, parsed.ios_srcs, "ios_srcs", metadata_path, false)?;
    let android_srcs = validate_repo_relative_paths(
        repo_root,
        parsed.android_srcs,
        "android_srcs",
        metadata_path,
        false,
    )?;

    Ok(ModuleManifest {
        kind,
        target_label: parsed.target_label,
        id: id.to_owned(),
        depends_on,
        schema_files,
        methods: parsed.methods,
        permissions: parsed.permissions,
        plist: parsed.plist,
        android_manifest: parsed.android_manifest,
        entitlements: parsed.entitlements,
        generated_sources: parsed.generated_sources,
        init_priority: parsed.init_priority,
        ios_srcs,
        android_srcs,
    })
}

fn validate_labels(
    labels: Vec<String>,
    field: &str,
    metadata_path: &Utf8Path,
) -> AtomResult<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut validated = Vec::with_capacity(labels.len());
    for label in labels {
        if !(label.starts_with("//") || label.starts_with('@')) {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("{field} entries must be absolute Bazel labels"),
                metadata_path.as_str(),
            ));
        }
        if !seen.insert(label.clone()) {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("duplicate dependency label: {label}"),
                metadata_path.as_str(),
            ));
        }
        validated.push(label);
    }
    Ok(validated)
}

fn validate_repo_relative_paths(
    repo_root: &Utf8Path,
    paths: Vec<String>,
    field: &str,
    metadata_path: &Utf8Path,
    require_non_empty: bool,
) -> AtomResult<Vec<Utf8PathBuf>> {
    if require_non_empty && paths.is_empty() {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            format!("{field} must declare at least one path"),
            metadata_path.as_str(),
        ));
    }

    let mut validated = Vec::with_capacity(paths.len());
    for raw in paths {
        if raw.is_empty()
            || raw
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("{field} entries must be normalized repo-relative paths"),
                metadata_path.as_str(),
            ));
        }

        let path = Utf8PathBuf::from(raw);
        if path.is_absolute() {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("{field} entries must be relative"),
                metadata_path.as_str(),
            ));
        }
        if !repo_root.join(&path).exists() {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleNotFound,
                format!("configured module input is missing: {path}"),
                metadata_path.as_str(),
            ));
        }
        validated.push(path);
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use atom_manifest::ModuleRequest;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{
        MethodSpec, ModuleKind, ModuleManifest, load_module_manifest_from_path,
        resolve_loaded_modules,
    };

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
                    plist: super::JsonMap::new(),
                    android_manifest: super::JsonMap::new(),
                    entitlements: super::JsonMap::new(),
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
                    plist: super::JsonMap::new(),
                    android_manifest: super::JsonMap::new(),
                    entitlements: super::JsonMap::new(),
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
