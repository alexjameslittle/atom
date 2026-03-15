use std::sync::{Arc, Mutex, MutexGuard};

use atom_runtime::{self, RuntimeEvent};

const DEFAULT_ROUTE: &str = "root";
const EVENT_SOURCE_ID: &str = "atom.navigation";
const CURRENT_ROUTE_KEY: &str = "atom.navigation.current_route";
const ROUTE_COUNT_KEY: &str = "atom.navigation.route_count";

#[derive(Debug, Clone, PartialEq, Eq)]
struct NavigationState {
    routes: Vec<String>,
}

impl NavigationState {
    fn current_route(&self) -> Option<String> {
        self.routes.last().cloned()
    }
}

/// Shared app-facing handle for navigation state owned by `Navigation`.
#[derive(Clone, Debug)]
pub struct NavigationHandle {
    state: Arc<Mutex<NavigationState>>,
}

impl NavigationHandle {
    pub fn push(&self, route: impl Into<String>) {
        let (current_route, route_count) = {
            let mut state = lock_state(&self.state);
            state.routes.push(route.into());
            (
                state
                    .current_route()
                    .unwrap_or_else(|| DEFAULT_ROUTE.to_owned()),
                state.routes.len(),
            )
        };
        record_route_change("push", current_route, route_count);
    }

    pub fn replace(&self, route: impl Into<String>) {
        let (current_route, route_count) = {
            let mut state = lock_state(&self.state);
            if let Some(current) = state.routes.last_mut() {
                *current = route.into();
            } else {
                state.routes.push(route.into());
            }
            (
                state
                    .current_route()
                    .unwrap_or_else(|| DEFAULT_ROUTE.to_owned()),
                state.routes.len(),
            )
        };
        record_route_change("replace", current_route, route_count);
    }

    #[must_use]
    pub fn pop(&self) -> Option<String> {
        let mut state = lock_state(&self.state);
        if state.routes.len() <= 1 {
            return None;
        }
        let popped = state.routes.pop();
        let current_route = state
            .current_route()
            .unwrap_or_else(|| DEFAULT_ROUTE.to_owned());
        let route_count = state.routes.len();
        drop(state);
        record_route_change("pop", current_route, route_count);
        popped
    }

    #[must_use]
    pub fn current_route(&self) -> Option<String> {
        lock_state(&self.state).current_route()
    }

    #[must_use]
    pub fn routes(&self) -> Vec<String> {
        lock_state(&self.state).routes.clone()
    }
}

/// Plain navigation state that can publish route changes through `atom_runtime::*`.
pub struct Navigation {
    state: Arc<Mutex<NavigationState>>,
}

impl Navigation {
    #[must_use]
    pub fn new(initial_route: impl Into<String>) -> Self {
        let initial_route = initial_route.into();
        let initial_route = if initial_route.is_empty() {
            DEFAULT_ROUTE.to_owned()
        } else {
            initial_route
        };
        Self {
            state: Arc::new(Mutex::new(NavigationState {
                routes: vec![initial_route],
            })),
        }
    }

    #[must_use]
    pub fn handle(&self) -> NavigationHandle {
        NavigationHandle {
            state: Arc::clone(&self.state),
        }
    }
}

fn record_route_change(action: &'static str, current_route: String, route_count: usize) {
    atom_runtime::set_state(CURRENT_ROUTE_KEY, &current_route);
    atom_runtime::set_state(ROUTE_COUNT_KEY, &route_count.to_string());
    atom_runtime::dispatch_event(RuntimeEvent::plugin(
        EVENT_SOURCE_ID,
        action,
        Some(current_route),
    ));
}

fn lock_state(state: &Arc<Mutex<NavigationState>>) -> MutexGuard<'_, NavigationState> {
    match state.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_ROUTE, Navigation};

    #[test]
    fn empty_initial_route_falls_back_to_root() {
        let navigation = Navigation::new("");
        assert_eq!(navigation.handle().routes(), vec![DEFAULT_ROUTE.to_owned()]);
    }

    #[test]
    fn handle_updates_stack() {
        let navigation = Navigation::new("home");
        let handle = navigation.handle();

        handle.push("details");
        handle.push("settings");

        assert_eq!(
            handle.routes(),
            vec![
                "home".to_owned(),
                "details".to_owned(),
                "settings".to_owned(),
            ]
        );
        assert_eq!(handle.current_route().as_deref(), Some("settings"));
    }

    #[test]
    fn pop_preserves_last_route() {
        let navigation = Navigation::new("home");
        let handle = navigation.handle();

        handle.push("details");
        assert_eq!(handle.pop().as_deref(), Some("details"));
        assert_eq!(handle.pop(), None);
        assert_eq!(handle.current_route().as_deref(), Some("home"));
    }

    #[test]
    fn replace_updates_current_route() {
        let navigation = Navigation::new("home");
        let handle = navigation.handle();

        handle.replace("profile");

        assert_eq!(handle.routes(), vec!["profile".to_owned()]);
        assert_eq!(handle.current_route().as_deref(), Some("profile"));
    }
}
