use std::env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub model: String,
    pub os: String,
}

#[must_use]
pub fn get() -> DeviceInfo {
    DeviceInfo {
        model: "atom-runtime".to_owned(),
        os: format!("{}-{}", env::consts::OS, env::consts::ARCH),
    }
}

#[cfg(test)]
mod tests {
    use super::get;

    #[test]
    fn get_returns_model_and_os() {
        let info = get();
        assert_eq!(info.model, "atom-runtime");
        assert!(info.os.contains('-'));
    }
}
