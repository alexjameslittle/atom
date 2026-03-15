/// Reserved for future runtime boot configuration.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig;

impl RuntimeConfig {
    pub fn builder() -> RuntimeConfigBuilder {
        RuntimeConfigBuilder::new()
    }
}

#[must_use]
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfigBuilder;

impl RuntimeConfigBuilder {
    pub fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn build(self) -> RuntimeConfig {
        RuntimeConfig
    }
}
