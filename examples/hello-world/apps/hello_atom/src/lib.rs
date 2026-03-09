use atom_runtime::RuntimeConfig;
use hello_world_lifecycle_logger::LifecycleLoggerPlugin;

#[must_use]
pub fn atom_runtime_config() -> RuntimeConfig {
    RuntimeConfig::builder()
        .plugin(LifecycleLoggerPlugin::new())
        .build()
}

#[must_use]
pub fn bootstrap_message() -> &'static str {
    "hello atom"
}
