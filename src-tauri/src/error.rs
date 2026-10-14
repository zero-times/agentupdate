use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn unsupported_platform() -> Self {
        Self {
            code: "UNSUPPORTED_PLATFORM",
            message: "此版本仅支持 macOS。".into(),
        }
    }
}
