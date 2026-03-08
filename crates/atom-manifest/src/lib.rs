mod bazel;
mod validate;

use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

pub use crate::bazel::{build_metadata_output, metadata_target};
use crate::validate::{
    RawDocument, validate_android, validate_app, validate_build, validate_ios, validate_modules,
};

pub const APP_METADATA_SUFFIX: &str = "_atom_app_metadata";
pub const MODULE_METADATA_SUFFIX: &str = "_atom_module_metadata";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub name: String,
    pub slug: String,
    pub entry_crate_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IosConfig {
    pub enabled: bool,
    pub bundle_id: Option<String>,
    pub deployment_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidConfig {
    pub enabled: bool,
    pub application_id: Option<String>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildConfig {
    pub generated_root: Utf8PathBuf,
    pub watch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRequest {
    pub target_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedManifest {
    pub repo_root: Utf8PathBuf,
    pub target_label: String,
    pub metadata_path: Utf8PathBuf,
    pub app: AppConfig,
    pub ios: IosConfig,
    pub android: AndroidConfig,
    pub build: BuildConfig,
    pub modules: Vec<ModuleRequest>,
}

/// # Errors
///
/// Returns an error if the manifest cannot be loaded, parsed, or validated.
pub fn load_manifest(repo_root: &Utf8Path, app_target: &str) -> AtomResult<NormalizedManifest> {
    let metadata_target = metadata_target(app_target, APP_METADATA_SUFFIX)?;
    let metadata_path = build_metadata_output(repo_root, &metadata_target)?;
    load_manifest_from_path(repo_root, app_target, &metadata_path)
}

fn load_manifest_from_path(
    repo_root: &Utf8Path,
    app_target: &str,
    metadata_path: &Utf8Path,
) -> AtomResult<NormalizedManifest> {
    if !metadata_path.exists() {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestNotFound,
            "atom app metadata file could not be found",
            metadata_path.as_str(),
        ));
    }

    let raw = fs::read_to_string(metadata_path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ManifestParseError,
            format!("failed to read app metadata: {error}"),
            metadata_path.as_str(),
        )
    })?;

    let parsed: RawDocument = serde_json::from_str(&raw).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::ManifestParseError,
            format!("failed to parse app metadata JSON: {error}"),
            metadata_path.as_str(),
        )
    })?;

    if parsed.kind != "atom_app" {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!("expected atom app metadata, found {}", parsed.kind),
            metadata_path.as_str(),
        ));
    }

    if parsed.target_label != app_target {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            format!(
                "app metadata target {} does not match requested target {}",
                parsed.target_label, app_target
            ),
            metadata_path.as_str(),
        ));
    }

    let app = validate_app(&parsed)?;
    let ios = validate_ios(parsed.ios)?;
    let android = validate_android(parsed.android)?;
    let build = validate_build(parsed.generated_root, parsed.watch)?;
    let modules = validate_modules(parsed.modules)?;

    if !ios.enabled && !android.enabled {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "at least one platform section must be enabled",
            metadata_path.as_str(),
        ));
    }

    Ok(NormalizedManifest {
        repo_root: repo_root.to_owned(),
        target_label: app_target.to_owned(),
        metadata_path: metadata_path.to_owned(),
        app,
        ios,
        android,
        build,
        modules,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::load_manifest_from_path;

    fn write_metadata(contents: &str) -> (tempfile::TempDir, Utf8PathBuf, Utf8PathBuf) {
        let directory = tempdir().expect("tempdir should exist");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        let metadata_path = root.join("hello_atom.atom.app.json");
        fs::write(&metadata_path, contents).expect("metadata should write");
        (directory, root, metadata_path)
    }

    #[test]
    fn loads_canonical_app_metadata() {
        let (_directory, root, metadata_path) = write_metadata(
            r#"{
  "kind": "atom_app",
  "target_label": "//apps/hello_atom:hello_atom",
  "name": "Hello Atom",
  "slug": "hello-atom",
  "entry_crate_label": "//apps/hello_atom:hello_atom",
  "generated_root": "generated",
  "watch": false,
  "ios": {
    "enabled": true,
    "bundle_id": "build.atom.hello",
    "deployment_target": "17.0"
  },
  "android": {
    "enabled": true,
    "application_id": "build.atom.hello",
    "min_sdk": 28,
    "target_sdk": 35
  },
  "modules": [
    "//modules/device_info:device_info"
  ]
}"#,
        );

        let manifest =
            load_manifest_from_path(&root, "//apps/hello_atom:hello_atom", &metadata_path)
                .expect("metadata should load");
        assert_eq!(manifest.app.slug, "hello-atom");
        assert_eq!(
            manifest.app.entry_crate_label,
            "//apps/hello_atom:hello_atom"
        );
        assert_eq!(manifest.modules.len(), 1);
    }

    #[test]
    fn rejects_unknown_top_level_keys() {
        let (_directory, root, metadata_path) = write_metadata(
            r#"{
  "kind": "atom_app",
  "target_label": "//apps/hello_atom:hello_atom",
  "name": "Hello Atom",
  "slug": "hello-atom",
  "entry_crate_label": "//apps/hello_atom:hello_atom",
  "unknown": true,
  "modules": []
}"#,
        );

        let error = load_manifest_from_path(&root, "//apps/hello_atom:hello_atom", &metadata_path)
            .expect_err("unknown key should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ManifestParseError);
    }

    #[test]
    fn rejects_relative_module_labels() {
        let (_directory, root, metadata_path) = write_metadata(
            r#"{
  "kind": "atom_app",
  "target_label": "//apps/hello_atom:hello_atom",
  "name": "Hello Atom",
  "slug": "hello-atom",
  "entry_crate_label": "//apps/hello_atom:hello_atom",
  "ios": {
    "enabled": true,
    "bundle_id": "build.atom.hello",
    "deployment_target": "17.0"
  },
  "modules": [
    ":device_info"
  ]
}"#,
        );

        let error = load_manifest_from_path(&root, "//apps/hello_atom:hello_atom", &metadata_path)
            .expect_err("relative module label should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ManifestInvalidValue);
    }

    #[test]
    fn supports_external_labels_when_deriving_metadata_target() {
        let target =
            crate::bazel::metadata_target("@vendor//modules/device_info:device_info", "_meta")
                .expect("external label should be supported");
        assert_eq!(target, "@vendor//modules/device_info:device_info_meta");
    }
}
