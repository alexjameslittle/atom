use camino::{Utf8Path, Utf8PathBuf};

use crate::{
    AndroidConfig, AppConfig, BuildConfig, ConfigPluginRequest, IosConfig, JsonMap, ModuleRequest,
    NormalizedManifest,
};

#[must_use]
pub fn fixture_manifest(repo_root: &Utf8Path) -> NormalizedManifest {
    NormalizedManifest {
        repo_root: repo_root.to_owned(),
        target_label: "//apps/fixture:fixture".to_owned(),
        metadata_path: repo_root.join("fixture.atom.app.json"),
        app: AppConfig {
            name: "Fixture".to_owned(),
            slug: "fixture".to_owned(),
            entry_crate_label: "//apps/fixture:fixture".to_owned(),
            entry_crate_name: "fixture".to_owned(),
        },
        ios: IosConfig {
            enabled: true,
            bundle_id: Some("build.atom.fixture".to_owned()),
            deployment_target: Some("17.0".to_owned()),
        },
        android: AndroidConfig {
            enabled: true,
            application_id: Some("build.atom.fixture".to_owned()),
            min_sdk: Some(28),
            target_sdk: Some(35),
        },
        build: BuildConfig {
            generated_root: Utf8PathBuf::from("generated"),
            watch: false,
        },
        modules: Vec::new(),
        config_plugins: Vec::new(),
    }
}

#[must_use]
pub fn fixture_config_plugin_request(id: &str, target_label: &str) -> ConfigPluginRequest {
    ConfigPluginRequest {
        target_label: target_label.to_owned(),
        id: id.to_owned(),
        atom_api_level: 1,
        min_atom_version: Some("0.1.0".to_owned()),
        ios_min_deployment_target: Some("17.0".to_owned()),
        android_min_sdk: Some(28),
        config: JsonMap::new(),
    }
}

#[must_use]
pub fn fixture_module_request(target_label: &str) -> ModuleRequest {
    ModuleRequest {
        target_label: target_label.to_owned(),
    }
}
