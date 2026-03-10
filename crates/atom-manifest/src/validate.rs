use std::collections::BTreeSet;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::Utf8PathBuf;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    AndroidConfig, AppConfig, BuildConfig, ConfigPluginRequest, IosConfig, JsonMap, ModuleRequest,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDocument {
    pub(crate) kind: String,
    pub(crate) target_label: String,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) entry_crate_label: String,
    pub(crate) entry_crate_name: String,
    pub(crate) generated_root: Option<String>,
    pub(crate) watch: Option<bool>,
    pub(crate) ios: Option<RawIos>,
    pub(crate) android: Option<RawAndroid>,
    #[serde(default)]
    pub(crate) modules: Vec<String>,
    #[serde(default)]
    pub(crate) config_plugins: Vec<RawConfigPlugin>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawIos {
    enabled: Option<bool>,
    bundle_id: Option<String>,
    deployment_target: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAndroid {
    enabled: Option<bool>,
    application_id: Option<String>,
    min_sdk: Option<u32>,
    target_sdk: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawConfigPlugin {
    id: String,
    target_label: String,
    atom_api_level: u32,
    min_atom_version: Option<String>,
    ios_min_deployment_target: Option<String>,
    android_min_sdk: Option<u32>,
    config: Map<String, Value>,
}

pub(crate) fn validate_app(raw: &RawDocument) -> AtomResult<AppConfig> {
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
    if !is_crate_name(&raw.entry_crate_name) {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "entry_crate_name must match ^[A-Za-z_][A-Za-z0-9_]*$",
            "entry_crate_name",
        ));
    }

    Ok(AppConfig {
        name: raw.name.clone(),
        slug: raw.slug.clone(),
        entry_crate_label: raw.entry_crate_label.clone(),
        entry_crate_name: raw.entry_crate_name.clone(),
    })
}

pub(crate) fn validate_ios(raw: Option<RawIos>) -> AtomResult<IosConfig> {
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

pub(crate) fn validate_android(raw: Option<RawAndroid>) -> AtomResult<AndroidConfig> {
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

pub(crate) fn validate_build(
    generated_root: Option<String>,
    watch: Option<bool>,
) -> AtomResult<BuildConfig> {
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

pub(crate) fn validate_modules(labels: Vec<String>) -> AtomResult<Vec<ModuleRequest>> {
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

pub(crate) fn validate_config_plugins(
    entries: Vec<RawConfigPlugin>,
) -> AtomResult<Vec<ConfigPluginRequest>> {
    let mut seen = BTreeSet::new();
    let mut plugins = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry.id.trim();
        if id.is_empty() {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "config_plugins entries must declare a non-empty id",
                "config_plugins.id",
            ));
        }
        validate_absolute_label(
            &entry.target_label,
            &format!("config_plugins.{id}.target_label"),
        )?;
        if !seen.insert(id.to_owned()) {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                format!("duplicate config plugin id: {id}"),
                format!("config_plugins.{id}.id"),
            ));
        }
        if let Some(min_atom_version) = &entry.min_atom_version
            && !is_semver(min_atom_version)
        {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "config_plugins.min_atom_version must match semver major.minor.patch",
                format!("config_plugins.{id}.min_atom_version"),
            ));
        }
        if let Some(ios_min_deployment_target) = &entry.ios_min_deployment_target
            && !is_deployment_target(ios_min_deployment_target)
        {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "config_plugins.ios_min_deployment_target must match ^[0-9]+\\.[0-9]+$",
                format!("config_plugins.{id}.ios_min_deployment_target"),
            ));
        }
        if let Some(android_min_sdk) = entry.android_min_sdk
            && android_min_sdk < 24
        {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "config_plugins.android_min_sdk must be >= 24",
                format!("config_plugins.{id}.android_min_sdk"),
            ));
        }
        plugins.push(ConfigPluginRequest {
            target_label: entry.target_label,
            id: id.to_owned(),
            atom_api_level: entry.atom_api_level,
            min_atom_version: entry.min_atom_version,
            ios_min_deployment_target: entry.ios_min_deployment_target,
            android_min_sdk: entry.android_min_sdk,
            config: JsonMap::from_iter(entry.config),
        });
    }
    Ok(plugins)
}

pub(crate) fn validate_absolute_label(value: &str, path: &str) -> AtomResult<()> {
    if value.starts_with("//") || value.starts_with('@') {
        Ok(())
    } else {
        Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "Bazel labels must be absolute",
            path,
        ))
    }
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

fn is_crate_name(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some(character) if character.is_ascii_alphabetic() || character == '_' => (),
        _ => return false,
    }

    characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
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
