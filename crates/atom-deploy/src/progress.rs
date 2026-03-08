use std::io::{self, IsTerminal};
use std::time::Duration;

use console::Style;
use indicatif::{ProgressBar, ProgressStyle};

pub struct DeployStep {
    bar: Option<ProgressBar>,
}

impl DeployStep {
    /// # Panics
    ///
    /// Panics if the built-in spinner template is invalid (should never happen).
    #[must_use]
    pub fn start(message: &str) -> Self {
        if !io::stdout().is_terminal() {
            return Self { bar: None };
        }

        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .expect("valid spinner template"),
        );
        bar.set_message(message.to_owned());
        bar.enable_steady_tick(Duration::from_millis(80));
        Self { bar: Some(bar) }
    }

    pub fn finish(self, message: &str) {
        if let Some(bar) = self.bar {
            let style = Style::new().green();
            bar.finish_with_message(format!("{} {message}", style.apply_to("✓")));
        }
    }

    pub fn fail(self, message: &str) {
        if let Some(bar) = self.bar {
            let style = Style::new().red();
            bar.finish_with_message(format!("{} {message}", style.apply_to("✗")));
        }
    }
}

/// Run a fallible operation with a spinner, finishing on success or failing on error.
///
/// # Errors
///
/// Returns the original error if the operation fails.
pub fn run_step<T>(
    progress_msg: &str,
    success_msg: &str,
    fail_msg: &str,
    op: impl FnOnce() -> atom_ffi::AtomResult<T>,
) -> atom_ffi::AtomResult<T> {
    let step = DeployStep::start(progress_msg);
    match op() {
        Ok(value) => {
            step.finish(success_msg);
            Ok(value)
        }
        Err(error) => {
            step.fail(fail_msg);
            Err(error)
        }
    }
}
