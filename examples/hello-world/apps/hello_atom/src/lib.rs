use atom_runtime::RuntimeConfig;

#[must_use]
pub fn atom_runtime_config() -> RuntimeConfig {
    RuntimeConfig::builder().build()
}

#[must_use]
pub fn bootstrap_message() -> &'static str {
    "hello atom"
}
