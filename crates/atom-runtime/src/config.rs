use std::sync::Arc;

use atom_ffi::AtomResult;

use crate::plugin::{PluginContext, RuntimePlugin};

/// Module init function type.
pub type ModuleInitFn = Box<dyn FnOnce(&PluginContext) -> AtomResult<()> + Send>;

pub type ModuleMethodHandler =
    Arc<dyn Fn(&PluginContext, &[u8]) -> AtomResult<Vec<u8>> + Send + Sync>;

#[derive(Clone)]
pub struct ModuleMethodRegistration {
    pub name: String,
    pub handler: ModuleMethodHandler,
}

impl ModuleMethodRegistration {
    #[must_use]
    pub fn new<F>(name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(&PluginContext, &[u8]) -> AtomResult<Vec<u8>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            handler: Arc::new(handler),
        }
    }
}

/// Registration for a module's lifecycle and runtime-call surface.
pub struct ModuleRegistration {
    pub id: String,
    pub init_order: usize,
    pub init_fn: ModuleInitFn,
    pub shutdown_fn: Option<Box<dyn FnOnce() + Send>>,
    pub methods: Vec<ModuleMethodRegistration>,
}

#[derive(Default)]
pub struct RuntimeConfig {
    pub plugins: Vec<Box<dyn RuntimePlugin>>,
    pub modules: Vec<ModuleRegistration>,
}

impl RuntimeConfig {
    pub fn builder() -> RuntimeConfigBuilder {
        RuntimeConfigBuilder::new()
    }
}

#[must_use]
pub struct RuntimeConfigBuilder {
    plugins: Vec<Box<dyn RuntimePlugin>>,
    modules: Vec<ModuleRegistration>,
}

impl RuntimeConfigBuilder {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            modules: Vec::new(),
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

    pub fn module(mut self, module: ModuleRegistration) -> Self {
        self.modules.push(module);
        self
    }

    #[must_use]
    pub fn build(self) -> RuntimeConfig {
        RuntimeConfig {
            plugins: self.plugins,
            modules: self.modules,
        }
    }
}

impl Default for RuntimeConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
