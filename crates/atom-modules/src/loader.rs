use std::collections::BTreeSet;
use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

use crate::{JsonMap, MethodSpec, ModuleKind, ModuleManifest};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawModuleManifest {
    kind: String,
    target_label: String,
    id: String,
    rust_crate_name: Option<String>,
    atom_api_level: u32,
    min_atom_version: Option<String>,
    ios_min_deployment_target: Option<String>,
    android_min_sdk: Option<u32>,
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

#[expect(
    clippy::too_many_lines,
    reason = "module manifest loading intentionally combines parse and validation in one entrypoint"
)]
pub(crate) fn load_module_manifest_from_path(
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
    match kind {
        ModuleKind::Rust => {
            let Some(crate_name) = parsed.rust_crate_name.as_deref() else {
                return Err(AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    "rust-authored modules must declare rust_crate_name",
                    metadata_path.as_str(),
                ));
            };
            if !is_crate_name(crate_name) {
                return Err(AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    "rust_crate_name must match ^[A-Za-z_][A-Za-z0-9_]*$",
                    metadata_path.as_str(),
                ));
            }
        }
        ModuleKind::Native => {
            if parsed.rust_crate_name.is_some() {
                return Err(AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    "native-only modules must not declare rust_crate_name",
                    metadata_path.as_str(),
                ));
            }
        }
    }
    if let Some(min_atom_version) = &parsed.min_atom_version
        && !is_semver(min_atom_version)
    {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            "min_atom_version must match semver major.minor.patch",
            metadata_path.as_str(),
        ));
    }
    if let Some(ios_min_deployment_target) = &parsed.ios_min_deployment_target
        && !is_deployment_target(ios_min_deployment_target)
    {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            "ios_min_deployment_target must match ^[0-9]+\\.[0-9]+$",
            metadata_path.as_str(),
        ));
    }
    if let Some(android_min_sdk) = parsed.android_min_sdk
        && android_min_sdk < 24
    {
        return Err(AtomError::with_path(
            AtomErrorCode::ModuleManifestInvalid,
            "android_min_sdk must be >= 24",
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
        rust_crate_name: parsed.rust_crate_name,
        atom_api_level: parsed.atom_api_level,
        min_atom_version: parsed.min_atom_version,
        ios_min_deployment_target: parsed.ios_min_deployment_target,
        android_min_sdk: parsed.android_min_sdk,
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

fn is_deployment_target(value: &str) -> bool {
    let mut components = value.split('.');
    let parts = [components.next(), components.next(), components.next()];
    matches!(
        parts,
        [Some(major), Some(minor), None]
            if !major.is_empty()
                && !minor.is_empty()
                && major.chars().all(|character| character.is_ascii_digit())
                && minor.chars().all(|character| character.is_ascii_digit())
    )
}

fn is_semver(value: &str) -> bool {
    let mut components = value.split('.');
    let parts = [
        components.next(),
        components.next(),
        components.next(),
        components.next(),
    ];
    matches!(
        parts,
        [Some(major), Some(minor), Some(patch), None]
            if !major.is_empty()
                && !minor.is_empty()
                && !patch.is_empty()
                && major.chars().all(|character| character.is_ascii_digit())
                && minor.chars().all(|character| character.is_ascii_digit())
                && patch.chars().all(|character| character.is_ascii_digit())
    )
}

fn is_crate_name(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
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
