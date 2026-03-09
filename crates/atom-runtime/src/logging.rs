use tracing_subscriber::EnvFilter;

/// Initialize the global tracing subscriber. Uses `try_init` so repeated calls
/// (e.g. in tests) are safe no-ops.
pub fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    #[cfg(target_os = "android")]
    {
        use tracing_subscriber::layer::SubscriberExt;
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_android::layer("AtomRuntime").expect("logcat layer")),
        );
    }

    #[cfg(not(target_os = "android"))]
    {
        use tracing_subscriber::fmt;
        let _ = fmt::Subscriber::builder()
            .with_env_filter(filter)
            .try_init();
    }
}
