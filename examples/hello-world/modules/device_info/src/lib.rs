use std::env;

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub model: String,
    pub os: String,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
}

#[atom_macros::atom_record]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    Loaded { info: DeviceInfo },
    Unavailable { reason: String },
}

#[must_use]
pub fn get() -> DeviceInfo {
    current_device_info()
}

#[must_use]
#[atom_macros::atom_export]
pub fn device_summary() -> String {
    let info = get();
    format!("{} ({})", info.model, info.os)
}

fn current_device_info() -> DeviceInfo {
    DeviceInfo {
        model: "atom-runtime".to_owned(),
        os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionStatus, device_summary, get};

    #[test]
    fn get_returns_model_and_os() {
        let info = get();
        assert_eq!(info.model, "atom-runtime");
        assert!(info.os.contains('-'));
    }

    #[test]
    fn annotated_export_uses_plain_rust_helper_without_manual_codecs() {
        let summary = device_summary();
        assert!(summary.contains("atom-runtime"));
        assert!(summary.contains('('));
        assert!(matches!(
            ConnectionStatus::Connected,
            ConnectionStatus::Connected
        ));
    }
}
