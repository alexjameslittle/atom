use atom_analytics::AnalyticsPlugin;
use atom_navigation::NavigationPlugin;
use atom_runtime::RuntimeConfig;
use device_info::runtime_module;
use hello_world_lifecycle_logger::LifecycleLoggerPlugin;

#[must_use]
pub fn atom_runtime_config() -> RuntimeConfig {
    let navigation = NavigationPlugin::new("home");
    let navigator = navigation.handle();
    navigator.push("device_info");

    let analytics = AnalyticsPlugin::new("hello_atom");
    let tracker = analytics.handle();
    tracker.track("runtime_configured");

    RuntimeConfig::builder()
        .module(runtime_module())
        .plugin(LifecycleLoggerPlugin::new())
        .plugin(navigation)
        .plugin(analytics)
        .build()
}

#[must_use]
pub fn bootstrap_message() -> &'static str {
    "hello atom"
}
