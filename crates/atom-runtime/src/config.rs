use std::any::{Any, type_name};
use std::marker::PhantomData;
use std::sync::Arc;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};

use crate::plugin::{PluginContext, RuntimePlugin};

/// Module init function type.
pub type ModuleInitFn = Box<dyn FnOnce(&PluginContext) -> AtomResult<()> + Send>;

pub(crate) struct ErasedModuleValue {
    value: Box<dyn Any + Send>,
    type_name: &'static str,
}

impl ErasedModuleValue {
    pub(crate) fn new<T>(value: T) -> Self
    where
        T: Send + 'static,
    {
        Self {
            value: Box::new(value),
            type_name: type_name::<T>(),
        }
    }

    pub(crate) fn type_name(&self) -> &'static str {
        self.type_name
    }

    pub(crate) fn downcast<T>(self) -> Result<T, Self>
    where
        T: Send + 'static,
    {
        match self.value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                value,
                type_name: self.type_name,
            }),
        }
    }
}

pub(crate) trait ErasedModuleMethodHandler: Send + Sync {
    fn call(
        &self,
        ctx: &PluginContext,
        request: ErasedModuleValue,
    ) -> AtomResult<ErasedModuleValue>;

    fn request_type_name(&self) -> &'static str;

    fn response_type_name(&self) -> &'static str;
}

struct TypedModuleMethodHandler<Request, Response, F> {
    handler: F,
    _marker: PhantomData<fn(Request) -> Response>,
}

impl<Request, Response, F> TypedModuleMethodHandler<Request, Response, F> {
    fn new(handler: F) -> Self {
        Self {
            handler,
            _marker: PhantomData,
        }
    }
}

impl<Request, Response, F> ErasedModuleMethodHandler
    for TypedModuleMethodHandler<Request, Response, F>
where
    Request: Send + 'static,
    Response: Send + 'static,
    F: Fn(&PluginContext, Request) -> AtomResult<Response> + Send + Sync + 'static,
{
    fn call(
        &self,
        ctx: &PluginContext,
        request: ErasedModuleValue,
    ) -> AtomResult<ErasedModuleValue> {
        let actual_request_type = request.type_name();
        let request = request.downcast::<Request>().map_err(|_| {
            AtomError::new(
                AtomErrorCode::BridgeInvalidArgument,
                format!(
                    "runtime module request type mismatch: expected {}, got {}",
                    type_name::<Request>(),
                    actual_request_type
                ),
            )
        })?;
        (self.handler)(ctx, request).map(ErasedModuleValue::new)
    }

    fn request_type_name(&self) -> &'static str {
        type_name::<Request>()
    }

    fn response_type_name(&self) -> &'static str {
        type_name::<Response>()
    }
}

pub type ModuleMethodHandler = Arc<dyn ErasedModuleMethodHandler>;

#[derive(Clone)]
pub struct ModuleMethodRegistration {
    pub name: String,
    pub(crate) handler: ModuleMethodHandler,
}

impl ModuleMethodRegistration {
    #[must_use]
    pub fn new<Request, Response, F>(name: impl Into<String>, handler: F) -> Self
    where
        Request: Send + 'static,
        Response: Send + 'static,
        F: Fn(&PluginContext, Request) -> AtomResult<Response> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            handler: Arc::new(TypedModuleMethodHandler::<Request, Response, F>::new(
                handler,
            )),
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
