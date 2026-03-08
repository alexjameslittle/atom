use std::collections::BTreeSet;
use std::fs;
use std::process::Command;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDocument {
    kind: String,
    target_label: String,
    name: String,
    slug: String,
    entry_crate_label: String,
    generated_root: Option<String>,
    watch: Option<bool>,
    ios: Option<RawIos>,
    android: Option<RawAndroid>,
    #[serde(default)]
    modules: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIos {
    enabled: Option<bool>,
    bundle_id: Option<String>,
    deployment_target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAndroid {
    enabled: Option<bool>,
    application_id: Option<String>,
    min_sdk: Option<u32>,
    target_sdk: Option<u32>,
}

pub fn metadata_target(label: &str, suffix: &str) -> AtomResult<String> {
    let (repository, rest) = if let Some(rest) = label.strip_prefix("//") {
        ("", rest)
    } else if let Some((repository, rest)) = label.split_once("//") {
        if repository.starts_with('@') && !repository[1..].is_empty() {
            (repository, rest)
        } else {
            return Err(AtomError::new(
                AtomErrorCode::CliUsageError,
                "atom targets must use absolute Bazel labels like //pkg:target",
            ));
        }
    } else {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "atom targets must use absolute Bazel labels like //pkg:target",
        ));
    };

    let (package, target) = match rest.split_once(':') {
        Some((package, target)) => (package, target),
        None => {
            let inferred = rest.rsplit('/').next().unwrap_or(rest);
            (rest, inferred)
        }
    };

    if target.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::CliUsageError,
            "atom targets must include a non-empty Bazel target name",
        ));
    }

    Ok(if package.is_empty() {
        format!("{repository}//:{target}{suffix}")
    } else {
        format!("{repository}//{package}:{target}{suffix}")
    })
}

pub fn build_metadata_output(repo_root: &Utf8Path, target: &str) -> AtomResult<Utf8PathBuf> {
    invoke_bazel(repo_root, &["build", target])?;
    let output = capture_bazel(repo_root, &["cquery", target, "--output=files"])?;
    let Some(first_line) = output.lines().find(|line| !line.trim().is_empty()) else {
        return Err(AtomError::new(
            AtomErrorCode::ManifestNotFound,
            format!("bazel did not return an output path for {target}"),
        ));
    };

    if first_line.starts_with('/') {
        Utf8PathBuf::from_path_buf(first_line.into()).map_err(|_| {
            AtomError::new(
                AtomErrorCode::ManifestParseError,
                "bazel returned a non-UTF-8 metadata path",
            )
        })
    } else {
        Ok(repo_root.join(first_line))
    }
}

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

fn validate_app(raw: &RawDocument) -> AtomResult<AppConfig> {
    if raw.name.trim().is_empty() {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "app name must be a non-empty UTF-8 string",
            "name",
        ));
    }
    if !is_slug(&raw.slug) {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "slug must match ^[a-z][a-z0-9-]{1,62}$",
            "slug",
        ));
    }
    validate_absolute_label(&raw.entry_crate_label, "entry_crate_label")?;

    Ok(AppConfig {
        name: raw.name.clone(),
        slug: raw.slug.clone(),
        entry_crate_label: raw.entry_crate_label.clone(),
    })
}

fn validate_ios(raw: Option<RawIos>) -> AtomResult<IosConfig> {
    let Some(raw) = raw else {
        return Ok(IosConfig {
            enabled: false,
            bundle_id: None,
            deployment_target: None,
        });
    };

    let enabled = raw.enabled.unwrap_or(true);
    if enabled {
        let bundle_id = raw.bundle_id.ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ManifestMissingField,
                "ios.bundle_id is required when ios is enabled",
                "ios.bundle_id",
            )
        })?;
        let deployment_target = raw.deployment_target.ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ManifestMissingField,
                "ios.deployment_target is required when ios is enabled",
                "ios.deployment_target",
            )
        })?;
        if !is_reverse_dns(&bundle_id) {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "ios.bundle_id must be a reverse-DNS identifier",
                "ios.bundle_id",
            ));
        }
        if !is_deployment_target(&deployment_target) {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "ios.deployment_target must match ^[0-9]+\\.[0-9]+$",
                "ios.deployment_target",
            ));
        }
        Ok(IosConfig {
            enabled,
            bundle_id: Some(bundle_id),
            deployment_target: Some(deployment_target),
        })
    } else {
        Ok(IosConfig {
            enabled: false,
            bundle_id: raw.bundle_id,
            deployment_target: raw.deployment_target,
        })
    }
}

fn validate_android(raw: Option<RawAndroid>) -> AtomResult<AndroidConfig> {
    let Some(raw) = raw else {
        return Ok(AndroidConfig {
            enabled: false,
            application_id: None,
            min_sdk: None,
            target_sdk: None,
        });
    };

    let enabled = raw.enabled.unwrap_or(true);
    if enabled {
        let application_id = raw.application_id.ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ManifestMissingField,
                "android.application_id is required when android is enabled",
                "android.application_id",
            )
        })?;
        let min_sdk = raw.min_sdk.ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ManifestMissingField,
                "android.min_sdk is required when android is enabled",
                "android.min_sdk",
            )
        })?;
        let target_sdk = raw.target_sdk.ok_or_else(|| {
            AtomError::with_path(
                AtomErrorCode::ManifestMissingField,
                "android.target_sdk is required when android is enabled",
                "android.target_sdk",
            )
        })?;
        if !is_reverse_dns(&application_id) {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "android.application_id must be a reverse-DNS identifier",
                "android.application_id",
            ));
        }
        if min_sdk < 24 {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "android.min_sdk must be >= 24",
                "android.min_sdk",
            ));
        }
        if target_sdk < min_sdk {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "android.target_sdk must be >= android.min_sdk",
                "android.target_sdk",
            ));
        }
        Ok(AndroidConfig {
            enabled,
            application_id: Some(application_id),
            min_sdk: Some(min_sdk),
            target_sdk: Some(target_sdk),
        })
    } else {
        Ok(AndroidConfig {
            enabled: false,
            application_id: raw.application_id,
            min_sdk: raw.min_sdk,
            target_sdk: raw.target_sdk,
        })
    }
}

fn validate_build(generated_root: Option<String>, watch: Option<bool>) -> AtomResult<BuildConfig> {
    let generated_root = generated_root.unwrap_or_else(|| "generated".to_owned());
    let root = Utf8PathBuf::from(generated_root.as_str());
    if root.is_absolute() {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "generated_root must be relative",
            "generated_root",
        ));
    }

    Ok(BuildConfig {
        generated_root: root,
        watch: watch.unwrap_or(false),
    })
}

fn validate_modules(labels: Vec<String>) -> AtomResult<Vec<ModuleRequest>> {
    let mut seen = BTreeSet::new();
    let mut modules = Vec::with_capacity(labels.len());
    for label in labels {
        validate_absolute_label(&label, "modules")?;
        if !seen.insert(label.clone()) {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                format!("duplicate module target: {label}"),
                label,
            ));
        }
        modules.push(ModuleRequest {
            target_label: label,
        });
    }
    Ok(modules)
}

fn validate_absolute_label(value: &str, path: &str) -> AtomResult<()> {
    if value.starts_with("//") || value.starts_with("@") {
        Ok(())
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "Bazel labels must be absolute",
            path,
        ))
    }
}

fn invoke_bazel(repo_root: &Utf8Path, args: &[&str]) -> AtomResult<()> {
    let status = Command::new("bazel")
        .args(args)
        .current_dir(repo_root)
        .status()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke bazel: {error}"),
            )
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("bazel {} exited with status {}", args.join(" "), status),
        ))
    }
}

fn capture_bazel(repo_root: &Utf8Path, args: &[&str]) -> AtomResult<String> {
    let output = Command::new("bazel")
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke bazel: {error}"),
            )
        })?;

    if !output.status.success() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "bazel {} exited with status {}",
                args.join(" "),
                output.status
            ),
        ));
    }

    String::from_utf8(output.stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ManifestParseError,
            "bazel returned non-UTF-8 output",
        )
    })
}

fn is_slug(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(character) if character.is_ascii_lowercase() => (),
        _ => return false,
    }

    let length = value.chars().count();
    if !(2..=63).contains(&length) {
        return false;
    }

    characters.all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    })
}

fn is_reverse_dns(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() < 2 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn is_deployment_target(value: &str) -> bool {
    let mut parts = value.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(major), Some(minor), None) if !major.is_empty()
            && !minor.is_empty()
            && major.chars().all(|character| character.is_ascii_digit())
            && minor.chars().all(|character| character.is_ascii_digit())
    )
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
        let target = super::metadata_target("@vendor//modules/device_info:device_info", "_meta")
            .expect("external label should be supported");
        assert_eq!(target, "@vendor//modules/device_info:device_info_meta");
    }
}
