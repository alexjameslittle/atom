use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;

/// Initialize the global tracing subscriber. Uses `try_init` so repeated calls
/// (e.g. in tests) are safe no-ops.
pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt::Subscriber::builder()
        .with_env_filter(filter)
        .try_init();
}
