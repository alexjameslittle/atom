use atom_cng::{
    BackendContribution, ConfigPlugin, ConfigPluginContext, ConfigPluginRegistry, ContributedFile,
    FileSource,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_manifest::ConfigPluginRequest;
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use serde_json::{Value, json};

const PLUGIN_ID: &str = "app_icon";
const IOS_RESOURCE_NAME: &str = "AppIcon.icon";
const IOS_RESOURCE_PREFIX: &str = "resources";
const ANDROID_RESOURCE_PATH: &str = "res/mipmap-xxxhdpi/ic_launcher.png";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppIconConfig {
    ios: Option<String>,
    android: Option<String>,
}

#[derive(Debug)]
struct AppIconPlugin {
    ios: Option<Utf8PathBuf>,
    android: Option<Utf8PathBuf>,
}

pub fn register(registry: &mut ConfigPluginRegistry) {
    registry.register(PLUGIN_ID, instantiate);
}

fn instantiate(entry: &ConfigPluginRequest) -> AtomResult<Box<dyn ConfigPlugin>> {
    let config: AppIconConfig = serde_json::from_value(Value::Object(entry.config.clone()))
        .map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                format!("failed to parse app_icon config: {error}"),
                format!("config_plugins.{}.config", entry.id),
            )
        })?;

    Ok(Box::new(AppIconPlugin {
        ios: config.ios.map(Utf8PathBuf::from),
        android: config.android.map(Utf8PathBuf::from),
    }))
}

impl ConfigPlugin for AppIconPlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn validate(&self) -> AtomResult<()> {
        if let Some(path) = &self.ios {
            validate_relative_path(path, "config_plugins.app_icon.config.ios")?;
            if path.extension() != Some("icon") {
                return Err(AtomError::with_path(
                    AtomErrorCode::ManifestInvalidValue,
                    "app_icon ios path must point to a .icon bundle",
                    "config_plugins.app_icon.config.ios",
                ));
            }
        }
        if let Some(path) = &self.android {
            validate_relative_path(path, "config_plugins.app_icon.config.android")?;
            if path.extension() != Some("png") {
                return Err(AtomError::with_path(
                    AtomErrorCode::ManifestInvalidValue,
                    "app_icon android path must point to a .png file",
                    "config_plugins.app_icon.config.android",
                ));
            }
        }
        Ok(())
    }

    fn contribute_backend(
        &self,
        backend_id: &str,
        ctx: &ConfigPluginContext<'_>,
    ) -> AtomResult<BackendContribution> {
        match backend_id {
            "ios" => self.contribute_ios(ctx),
            "android" => self.contribute_android(ctx),
            _ => Ok(BackendContribution::default()),
        }
    }
}

impl AppIconPlugin {
    fn contribute_ios(&self, ctx: &ConfigPluginContext<'_>) -> AtomResult<BackendContribution> {
        let Some(source) = &self.ios else {
            return Ok(BackendContribution::default());
        };
        let source_path = ctx.repo_root.join(source);
        if !source_path.is_dir() {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "app_icon ios path must point to an existing .icon directory",
                "config_plugins.app_icon.config.ios",
            ));
        }
        if !source_path.join("icon.json").exists() {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "app_icon ios bundle must contain icon.json",
                "config_plugins.app_icon.config.ios",
            ));
        }

        Ok(BackendContribution {
            files: vec![ContributedFile {
                source: FileSource::Copy(source.clone()),
                output: ctx
                    .generated_root
                    .join("ios")
                    .join(&ctx.app.slug)
                    .join(IOS_RESOURCE_PREFIX)
                    .join(IOS_RESOURCE_NAME),
            }],
            metadata_entries: json_object(json!({
                "CFBundleIconName": "AppIcon"
            })),
            bazel_resources: Vec::new(),
            bazel_resource_globs: vec![format!("{IOS_RESOURCE_PREFIX}/{IOS_RESOURCE_NAME}/**")],
        })
    }

    fn contribute_android(&self, ctx: &ConfigPluginContext<'_>) -> AtomResult<BackendContribution> {
        let Some(source) = &self.android else {
            return Ok(BackendContribution::default());
        };
        let source_path = ctx.repo_root.join(source);
        if !source_path.is_file() {
            return Err(AtomError::with_path(
                AtomErrorCode::ManifestInvalidValue,
                "app_icon android path must point to an existing .png file",
                "config_plugins.app_icon.config.android",
            ));
        }

        Ok(BackendContribution {
            files: vec![ContributedFile {
                source: FileSource::Copy(source.clone()),
                output: ctx
                    .generated_root
                    .join("android")
                    .join(&ctx.app.slug)
                    .join(ANDROID_RESOURCE_PATH),
            }],
            metadata_entries: json_object(json!({
                "application": {
                    "@android:icon": "@mipmap/ic_launcher"
                }
            })),
            bazel_resources: vec![ANDROID_RESOURCE_PATH.to_owned()],
            bazel_resource_globs: Vec::new(),
        })
    }
}

fn json_object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn validate_relative_path(path: &Utf8Path, field: &str) -> AtomResult<()> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, camino::Utf8Component::ParentDir))
    {
        return Err(AtomError::with_path(
            AtomErrorCode::ManifestInvalidValue,
            "config plugin paths must be normalized repo-relative paths",
            field,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::OnceLock;

    use atom_cng::{ConfigPlugin, ConfigPluginContext};
    use atom_manifest::{AppConfig, ConfigPluginRequest};
    use camino::{Utf8Path, Utf8PathBuf};
    use serde_json::json;
    use tempfile::tempdir;

    use super::{AppIconPlugin, instantiate};

    fn plugin_request(config: serde_json::Map<String, serde_json::Value>) -> ConfigPluginRequest {
        ConfigPluginRequest {
            target_label: "//crates/atom-cng-app-icon:atom-cng-app-icon".to_owned(),
            id: "app_icon".to_owned(),
            atom_api_level: 1,
            min_atom_version: Some("0.1.0".to_owned()),
            ios_min_deployment_target: Some("18.0".to_owned()),
            android_min_sdk: Some(28),
            config,
        }
    }

    fn context(root: &Utf8PathBuf) -> ConfigPluginContext<'_> {
        static APP: OnceLock<AppConfig> = OnceLock::new();
        ConfigPluginContext {
            app: APP.get_or_init(|| AppConfig {
                name: "Hello Atom".to_owned(),
                slug: "hello-atom".to_owned(),
                entry_crate_label: "//apps/hello_atom:hello_atom".to_owned(),
                entry_crate_name: "hello_atom".to_owned(),
            }),
            repo_root: root,
            generated_root: Utf8Path::new("generated"),
        }
    }

    #[test]
    fn instantiate_parses_config() {
        let plugin = instantiate(&plugin_request(json_object(json!({
            "ios": "assets/AppIcon.icon",
            "android": "assets/ic_launcher.png"
        }))))
        .expect("plugin should parse");
        assert_eq!(plugin.id(), "app_icon");
    }

    #[test]
    fn validate_rejects_absolute_paths() {
        let plugin = AppIconPlugin {
            ios: Some(Utf8PathBuf::from("/tmp/AppIcon.icon")),
            android: None,
        };
        let error = plugin.validate().expect_err("absolute path should fail");
        assert_eq!(error.code, atom_ffi::AtomErrorCode::ManifestInvalidValue);
    }

    #[test]
    fn contributes_expected_outputs() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("assets/AppIcon.icon")).expect("icon dir");
        fs::create_dir_all(root.join("assets/AppIcon.icon/Assets")).expect("icon assets dir");
        fs::write(
            root.join("assets/AppIcon.icon/icon.json"),
            "{\"name\":\"AppIcon\"}",
        )
        .expect("icon json");
        fs::write(root.join("assets/AppIcon.icon/Assets/atom.svg"), "<svg />").expect("icon svg");
        fs::write(root.join("assets/ic_launcher.png"), "png").expect("png");

        let plugin = instantiate(&plugin_request(json_object(json!({
            "ios": "assets/AppIcon.icon",
            "android": "assets/ic_launcher.png"
        }))))
        .expect("plugin");
        let ios = plugin
            .contribute_backend("ios", &context(&root))
            .expect("ios contribution");
        let android = plugin
            .contribute_backend("android", &context(&root))
            .expect("android contribution");

        assert_eq!(
            ios.files[0].output,
            Utf8PathBuf::from("generated/ios/hello-atom/resources/AppIcon.icon")
        );
        assert_eq!(
            android.files[0].output,
            Utf8PathBuf::from("generated/android/hello-atom/res/mipmap-xxxhdpi/ic_launcher.png")
        );
        assert!(ios.bazel_resources.is_empty());
        assert_eq!(
            ios.bazel_resource_globs,
            vec!["resources/AppIcon.icon/**".to_owned()]
        );
        assert_eq!(
            android.bazel_resources,
            vec!["res/mipmap-xxxhdpi/ic_launcher.png".to_owned()]
        );
    }

    fn json_object(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        }
    }
}
