use crate::agents::{discovery::Installation, versions::AvailableVersion};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Queued,
    Preflight,
    BackingUp,
    Prepared,
    Installing,
    Verifying,
    Succeeded,
    PreparationFailed,
    NeedsRecovery,
    Recovering,
    Restored,
    RecoveryFailed,
    Cancelled,
}
impl Stage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preflight => "preflight",
            Self::BackingUp => "backing_up",
            Self::Prepared => "prepared",
            Self::Installing => "installing",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::PreparationFailed => "preparation_failed",
            Self::NeedsRecovery => "needs_recovery",
            Self::Recovering => "recovering",
            Self::Restored => "restored",
            Self::RecoveryFailed => "recovery_failed",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn allows(self, next: Self) -> bool {
        use Stage::*;
        matches!(
            (self, next),
            (Queued, Preflight | Cancelled)
                | (Preflight, BackingUp | PreparationFailed | Cancelled)
                | (BackingUp, Prepared | PreparationFailed | Cancelled)
                | (Prepared, Installing | PreparationFailed | Cancelled)
                | (Installing, Verifying | NeedsRecovery)
                | (Verifying, Succeeded | NeedsRecovery)
                | (Succeeded | NeedsRecovery | RecoveryFailed, Recovering)
                | (Recovering, Restored | RecoveryFailed)
        )
    }
    pub fn incomplete_mutation(self) -> bool {
        matches!(self, Self::Installing | Self::Verifying | Self::Recovering)
    }
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Preflight
                | Self::BackingUp
                | Self::Prepared
                | Self::Installing
                | Self::Verifying
                | Self::NeedsRecovery
                | Self::Recovering
                | Self::RecoveryFailed
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedItem {
    pub installation: Installation,
    pub target: AvailableVersion,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTransaction {
    pub id: String,
    pub plan_id: String,
    pub ordinal: u32,
    pub installation: Installation,
    pub target: AvailableVersion,
    pub stage: Stage,
    pub restore_point_id: Option<String>,
    pub failure_code: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlan {
    pub id: String,
    pub state: String,
    #[serde(default)]
    pub cancel_requested: bool,
    pub created_at: u64,
    pub transactions: Vec<UpdateTransaction>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePoint {
    pub id: String,
    pub transaction_id: String,
    pub relative_path: String,
    pub manifest_sha256: String,
    pub created_at: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestHeader {
    pub schema_version: u32,
    pub id: String,
    pub transaction_id: String,
    pub installation_id: String,
    pub original_version: String,
    pub target_version: String,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    pub transaction_id: String,
    pub code: String,
    pub created_at: u64,
}
