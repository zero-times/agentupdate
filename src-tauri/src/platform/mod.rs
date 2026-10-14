use crate::error::AppError;
use serde::Serialize;
pub mod files;
pub mod process;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformInfo {
    pub os: &'static str,
    pub architecture: &'static str,
    pub minimum_system_version: &'static str,
}

// Future OS implementations belong here, not in individual Agent adapters.
pub fn current() -> Result<PlatformInfo, AppError> {
    if !cfg!(target_os = "macos") {
        return Err(AppError::unsupported_platform());
    }
    Ok(PlatformInfo {
        os: "macos",
        architecture: std::env::consts::ARCH,
        minimum_system_version: "15.0",
    })
}

pub mod locks;

pub mod network;
