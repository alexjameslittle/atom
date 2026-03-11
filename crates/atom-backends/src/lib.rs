use atom_ffi::{AtomError, AtomErrorCode, AtomResult};

pub trait BackendDefinition {
    fn id(&self) -> &'static str;
    fn platform(&self) -> &'static str;
}

impl<B> BackendDefinition for Box<B>
where
    B: BackendDefinition + ?Sized,
{
    fn id(&self) -> &'static str {
        (**self).id()
    }

    fn platform(&self) -> &'static str {
        (**self).platform()
    }
}

pub struct BackendRegistry<B> {
    backends: Vec<B>,
}

impl<B> Default for BackendRegistry<B> {
    fn default() -> Self {
        Self {
            backends: Vec::new(),
        }
    }
}

impl<B> BackendRegistry<B>
where
    B: BackendDefinition,
{
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// # Errors
    ///
    /// Returns an error if a backend with the same id is already registered.
    pub fn register(&mut self, backend: B) -> AtomResult<()> {
        let id = backend.id();
        if self.backends.iter().any(|existing| existing.id() == id) {
            return Err(AtomError::new(
                AtomErrorCode::InternalBug,
                format!("backend registry already contains id {id}"),
            ));
        }
        self.backends.push(backend);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&B> {
        self.backends.iter().find(|backend| backend.id() == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &B> {
        self.backends.iter()
    }
}

#[cfg(test)]
mod tests {
    use atom_ffi::AtomErrorCode;

    use super::{BackendDefinition, BackendRegistry};

    struct FixtureBackend {
        id: &'static str,
        platform: &'static str,
    }

    impl BackendDefinition for FixtureBackend {
        fn id(&self) -> &'static str {
            self.id
        }

        fn platform(&self) -> &'static str {
            self.platform
        }
    }

    #[test]
    fn duplicate_backend_ids_are_rejected() {
        let mut registry = BackendRegistry::new();
        registry
            .register(FixtureBackend {
                id: "ios",
                platform: "ios",
            })
            .expect("first registration should succeed");

        let error = registry
            .register(FixtureBackend {
                id: "ios",
                platform: "ios",
            })
            .expect_err("duplicate id should fail");

        assert_eq!(error.code, AtomErrorCode::InternalBug);
        assert!(
            error
                .message
                .contains("backend registry already contains id ios")
        );
    }
}
