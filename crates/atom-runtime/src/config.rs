use crate::plugin::RuntimePlugin;

#[derive(Default)]
pub struct RuntimeConfig {
    pub plugins: Vec<Box<dyn RuntimePlugin>>,
}

impl RuntimeConfig {
    pub fn builder() -> RuntimeConfigBuilder {
        RuntimeConfigBuilder::new()
    }
}

#[must_use]
pub struct RuntimeConfigBuilder {
    plugins: Vec<Box<dyn RuntimePlugin>>,
}

impl RuntimeConfigBuilder {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn plugin<P>(mut self, plugin: P) -> Self
    where
        P: RuntimePlugin + 'static,
    {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub fn boxed_plugin(mut self, plugin: Box<dyn RuntimePlugin>) -> Self {
        self.plugins.push(plugin);
        self
    }

    #[must_use]
    pub fn build(self) -> RuntimeConfig {
        RuntimeConfig {
            plugins: self.plugins,
        }
    }
}

impl Default for RuntimeConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
