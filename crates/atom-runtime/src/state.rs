use atom_ffi::{AtomError, AtomErrorCode, AtomLifecycleEvent, AtomResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Created,
    Initializing,
    Running,
    Backgrounded,
    Suspended,
    Terminating,
    Terminated,
    Failed,
}

/// Pure transition validator. Returns the target state for a valid transition,
/// or `RUNTIME_TRANSITION_INVALID` for an invalid one.
pub fn validate_transition(
    from: RuntimeState,
    event: AtomLifecycleEvent,
) -> AtomResult<RuntimeState> {
    match (from, event) {
        (RuntimeState::Running, AtomLifecycleEvent::Background) => Ok(RuntimeState::Backgrounded),
        // iOS fires sceneWillEnterForeground on initial launch when the runtime is already
        // Running, so treat Foreground while Running as a no-op.
        (RuntimeState::Running, AtomLifecycleEvent::Foreground) => Ok(RuntimeState::Running),
        (RuntimeState::Backgrounded, AtomLifecycleEvent::Foreground)
        | (RuntimeState::Suspended, AtomLifecycleEvent::Resume) => Ok(RuntimeState::Running),
        (RuntimeState::Backgrounded, AtomLifecycleEvent::Suspend) => Ok(RuntimeState::Suspended),
        (
            RuntimeState::Running | RuntimeState::Backgrounded | RuntimeState::Suspended,
            AtomLifecycleEvent::Terminate,
        ) => Ok(RuntimeState::Terminating),
        (RuntimeState::Terminated | RuntimeState::Failed, _) => Err(AtomError::new(
            AtomErrorCode::RuntimeTransitionInvalid,
            "runtime cannot transition from a terminal state",
        )),
        _ => Err(AtomError::new(
            AtomErrorCode::RuntimeTransitionInvalid,
            format!("invalid transition from {from:?} with {event:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atom_ffi::AtomLifecycleEvent;

    #[test]
    fn valid_running_to_backgrounded() {
        assert_eq!(
            validate_transition(RuntimeState::Running, AtomLifecycleEvent::Background).unwrap(),
            RuntimeState::Backgrounded,
        );
    }

    #[test]
    fn valid_backgrounded_to_running() {
        assert_eq!(
            validate_transition(RuntimeState::Backgrounded, AtomLifecycleEvent::Foreground)
                .unwrap(),
            RuntimeState::Running,
        );
    }

    #[test]
    fn valid_backgrounded_to_suspended() {
        assert_eq!(
            validate_transition(RuntimeState::Backgrounded, AtomLifecycleEvent::Suspend).unwrap(),
            RuntimeState::Suspended,
        );
    }

    #[test]
    fn valid_suspended_to_running() {
        assert_eq!(
            validate_transition(RuntimeState::Suspended, AtomLifecycleEvent::Resume).unwrap(),
            RuntimeState::Running,
        );
    }

    #[test]
    fn valid_running_to_terminating() {
        assert_eq!(
            validate_transition(RuntimeState::Running, AtomLifecycleEvent::Terminate).unwrap(),
            RuntimeState::Terminating,
        );
    }

    #[test]
    fn valid_backgrounded_to_terminating() {
        assert_eq!(
            validate_transition(RuntimeState::Backgrounded, AtomLifecycleEvent::Terminate).unwrap(),
            RuntimeState::Terminating,
        );
    }

    #[test]
    fn valid_suspended_to_terminating() {
        assert_eq!(
            validate_transition(RuntimeState::Suspended, AtomLifecycleEvent::Terminate).unwrap(),
            RuntimeState::Terminating,
        );
    }

    #[test]
    fn invalid_terminated_rejects_all() {
        for event in [
            AtomLifecycleEvent::Foreground,
            AtomLifecycleEvent::Background,
            AtomLifecycleEvent::Suspend,
            AtomLifecycleEvent::Resume,
            AtomLifecycleEvent::Terminate,
        ] {
            let err = validate_transition(RuntimeState::Terminated, event).unwrap_err();
            assert_eq!(err.code, AtomErrorCode::RuntimeTransitionInvalid);
        }
    }

    #[test]
    fn invalid_failed_rejects_all() {
        for event in [
            AtomLifecycleEvent::Foreground,
            AtomLifecycleEvent::Background,
            AtomLifecycleEvent::Suspend,
            AtomLifecycleEvent::Resume,
            AtomLifecycleEvent::Terminate,
        ] {
            let err = validate_transition(RuntimeState::Failed, event).unwrap_err();
            assert_eq!(err.code, AtomErrorCode::RuntimeTransitionInvalid);
        }
    }

    #[test]
    fn invalid_created_to_backgrounded() {
        let err =
            validate_transition(RuntimeState::Created, AtomLifecycleEvent::Background).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::RuntimeTransitionInvalid);
    }

    #[test]
    fn foreground_while_running_is_noop() {
        // iOS fires sceneWillEnterForeground on initial launch, so Foreground from
        // Running must be accepted as a no-op rather than returning an error.
        assert_eq!(
            validate_transition(RuntimeState::Running, AtomLifecycleEvent::Foreground).unwrap(),
            RuntimeState::Running,
        );
    }

    #[test]
    fn invalid_running_to_initializing_via_resume() {
        let err =
            validate_transition(RuntimeState::Running, AtomLifecycleEvent::Resume).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::RuntimeTransitionInvalid);
    }

    #[test]
    fn invalid_running_to_suspended() {
        let err =
            validate_transition(RuntimeState::Running, AtomLifecycleEvent::Suspend).unwrap_err();
        assert_eq!(err.code, AtomErrorCode::RuntimeTransitionInvalid);
    }
}
