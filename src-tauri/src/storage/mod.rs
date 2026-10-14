pub mod history;
pub mod model;
use crate::{error::AppError, platform::files};
use model::*;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatus {
    pub state: &'static str,
}
pub fn status() -> StorageStatus {
    StorageStatus { state: "available" }
}
pub struct Store {
    pub root: PathBuf,
    connection: Connection,
}
fn db_error(_: rusqlite::Error) -> AppError {
    AppError::new("DATABASE_OPERATION", "本地事务记录操作失败；未确认成功。")
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn validate_id(id: &str) -> Result<(), AppError> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| AppError::new("INVALID_ID", "记录标识无效。"))
}
fn encode<T: serde::Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value).map_err(|_| AppError::new("DATABASE_RECORD", "无法保存事务记录。"))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_str(value).map_err(|_| {
        AppError::new(
            "DATABASE_RECORD_INVALID",
            "事务记录损坏；请保留数据并检查恢复材料。",
        )
    })
}

impl Store {
    pub fn open(root: &Path) -> Result<Self, AppError> {
        files::private_directory(root)?;
        let root = fs::canonicalize(root)
            .map_err(|_| AppError::new("DATA_DIRECTORY", "无法确认数据目录。"))?;
        for directory in ["restores", "logs", "locks"] {
            files::private_directory(&root.join(directory))?;
        }
        let database = root.join("state.sqlite3");
        if database.exists() || fs::symlink_metadata(&database).is_ok() {
            files::regular_private_file(&database)?;
            let mut file = fs::File::open(&database)
                .map_err(|_| AppError::new("DATABASE_OPEN", "无法读取数据库。"))?;
            let length = file
                .metadata()
                .map_err(|_| AppError::new("DATABASE_OPEN", "无法读取数据库。"))?
                .len();
            if length > 0 {
                let mut magic = [0u8; 16];
                if file.read_exact(&mut magic).is_err() || &magic != b"SQLite format 3\0" {
                    return Err(AppError::new(
                        "DATABASE_CORRUPT",
                        "数据库格式损坏；原文件已保留，未重建。",
                    ));
                }
            }
        } else {
            let file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&database)
                .map_err(|_| AppError::new("DATABASE_OPEN", "无法创建数据库。"))?;
            file.sync_all()
                .map_err(|_| AppError::new("DATA_SYNC", "无法确认数据库文件已落盘。"))?;
            files::sync_directory(&root)?;
        }
        let mut connection = Connection::open(&database)
            .map_err(|_| AppError::new("DATABASE_OPEN", "数据库无法打开；已保留原文件。"))?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        let integrity: String = connection
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|_| AppError::new("DATABASE_CORRUPT", "数据库完整性检查失败；未重建。"))?;
        if integrity != "ok" {
            return Err(AppError::new(
                "DATABASE_CORRUPT",
                "数据库完整性检查失败；未重建。",
            ));
        }
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_error)?;
        if version > 2 {
            return Err(AppError::new(
                "DATABASE_NEWER",
                "数据库来自较新版本，不能自动降级。",
            ));
        }
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
            )
            .map_err(db_error)?;
        if version == 0 {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            transaction.execute_batch("CREATE TABLE IF NOT EXISTS installations(id TEXT PRIMARY KEY, body TEXT NOT NULL, updated_at INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS plans(id TEXT PRIMARY KEY, state TEXT NOT NULL, created_at INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS transactions(id TEXT PRIMARY KEY, plan_id TEXT NOT NULL REFERENCES plans(id), instance_id TEXT NOT NULL REFERENCES installations(id), ordinal INTEGER NOT NULL, stage TEXT NOT NULL, restore_id TEXT, body TEXT NOT NULL, updated_at INTEGER NOT NULL);
                CREATE TABLE IF NOT EXISTS restore_points(id TEXT PRIMARY KEY, transaction_id TEXT NOT NULL REFERENCES transactions(id), body TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS events(id INTEGER PRIMARY KEY AUTOINCREMENT, transaction_id TEXT NOT NULL REFERENCES transactions(id), code TEXT NOT NULL, created_at INTEGER NOT NULL);
                CREATE INDEX IF NOT EXISTS transactions_instance ON transactions(instance_id,stage);
                PRAGMA user_version=1;").map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
        }
        if version < 2 {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            transaction.execute_batch("ALTER TABLE plans ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=2;").map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
        }
        for suffix in ["", "-wal", "-shm"] {
            let path = root.join(format!("state.sqlite3{suffix}"));
            if path.exists() {
                files::regular_private_file(&path)?;
            }
        }
        Ok(Self { root, connection })
    }

    pub fn create_plan(&mut self, items: Vec<PlannedItem>) -> Result<UpdatePlan, AppError> {
        if items.is_empty() || items.len() > 32 {
            return Err(AppError::new(
                "INVALID_PLAN",
                "更新计划必须包含 1 至 32 个安装实例。",
            ));
        }
        let mut unique = std::collections::HashSet::new();
        for item in &items {
            if !item.installation.supported
                || !item.installation.version_verified
                || !unique.insert(item.installation.id.clone())
            {
                return Err(AppError::new("INVALID_PLAN", "实例不支持更新或重复选择。"));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let created = now();
        let mut transactions = Vec::new();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        for item in &items {
            let mut statement = transaction
                .prepare("SELECT stage FROM transactions WHERE instance_id=?1")
                .map_err(db_error)?;
            for row in statement
                .query_map([&item.installation.id], |r| r.get::<_, String>(0))
                .map_err(db_error)?
            {
                let stage: Stage =
                    decode(&serde_json::to_string(&row.map_err(db_error)?).unwrap())?;
                if stage.active() {
                    return Err(AppError::new(
                        "INSTANCE_BUSY",
                        "该实例已有未完成任务，请先处理或取消。",
                    ));
                }
            }
        }
        transaction
            .execute(
                "INSERT INTO plans(id,state,created_at)VALUES(?1,'ready',?2)",
                params![id, created],
            )
            .map_err(db_error)?;
        for (ordinal, item) in items.into_iter().enumerate() {
            transaction.execute("INSERT INTO installations(id,body,updated_at)VALUES(?1,?2,?3)ON CONFLICT(id)DO UPDATE SET body=excluded.body,updated_at=excluded.updated_at",params![item.installation.id,encode(&item.installation)?,created]).map_err(db_error)?;
            let value = UpdateTransaction {
                id: uuid::Uuid::new_v4().to_string(),
                plan_id: id.clone(),
                ordinal: ordinal as u32,
                installation: item.installation,
                target: item.target,
                stage: Stage::Queued,
                restore_point_id: None,
                failure_code: None,
                created_at: created,
                updated_at: created,
            };
            transaction.execute("INSERT INTO transactions(id,plan_id,instance_id,ordinal,stage,body,updated_at)VALUES(?1,?2,?3,?4,?5,?6,?7)",params![value.id,id,value.installation.id,ordinal as u32,value.stage.key(),encode(&value)?,created]).map_err(db_error)?;
            transaction
                .execute(
                    "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,'queued',?2)",
                    params![value.id, created],
                )
                .map_err(db_error)?;
            transactions.push(value);
        }
        transaction.commit().map_err(db_error)?;
        Ok(UpdatePlan {
            id,
            state: "ready".into(),
            cancel_requested: false,
            created_at: created,
            transactions,
        })
    }

    pub fn transaction(&self, id: &str) -> Result<UpdateTransaction, AppError> {
        validate_id(id)?;
        let data: Option<(String, String, String, String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT body,stage,plan_id,instance_id,restore_id FROM transactions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(db_error)?;
        let (body, stage, plan, instance, restore) =
            data.ok_or_else(|| AppError::new("TRANSACTION_NOT_FOUND", "任务记录不存在。"))?;
        let value: UpdateTransaction = decode(&body)?;
        if value.id != id
            || value.stage.key() != stage
            || value.plan_id != plan
            || value.installation.id != instance
            || value.restore_point_id != restore
        {
            return Err(AppError::new(
                "DATABASE_RECORD_INVALID",
                "事务字段不一致；不能确认任务成功。",
            ));
        }
        Ok(value)
    }

    pub fn plans(&self) -> Result<Vec<UpdatePlan>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id,state,created_at,cancel_requested FROM plans ORDER BY created_at DESC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u64>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            })
            .map_err(db_error)?;
        let mut plans = Vec::new();
        for row in rows {
            let (id, state, created_at, cancel_requested) = row.map_err(db_error)?;
            let mut values = self
                .connection
                .prepare("SELECT id FROM transactions WHERE plan_id=?1 ORDER BY ordinal")
                .map_err(db_error)?;
            let mut transactions = Vec::new();
            for value in values
                .query_map([&id], |r| r.get::<_, String>(0))
                .map_err(db_error)?
            {
                transactions.push(self.transaction(&value.map_err(db_error)?)?);
            }
            plans.push(UpdatePlan {
                id,
                state,
                cancel_requested,
                created_at,
                transactions,
            });
        }
        Ok(plans)
    }

    pub fn plan(&self, id: &str) -> Result<UpdatePlan, AppError> {
        validate_id(id)?;
        self.plans()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::new("PLAN_NOT_FOUND", "更新计划不存在。"))
    }
    pub fn request_cancel(&mut self, id: &str) -> Result<UpdatePlan, AppError> {
        let plan = self.plan(id)?;
        if ["finished", "cancelled"].contains(&plan.state.as_str()) {
            return Ok(plan);
        }
        self.connection
            .execute("UPDATE plans SET cancel_requested=1 WHERE id=?1", [id])
            .map_err(db_error)?;
        if plan.state != "running" {
            self.cancel_queued(id)?;
        }
        self.plan(id)
    }
    pub fn cancel_requested(&self, id: &str) -> Result<bool, AppError> {
        validate_id(id)?;
        self.connection
            .query_row(
                "SELECT cancel_requested FROM plans WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db_error)
    }
    pub fn cancel_queued(&mut self, id: &str) -> Result<(), AppError> {
        for tx in self
            .plan(id)?
            .transactions
            .into_iter()
            .filter(|t| t.stage == Stage::Queued)
        {
            self.transition(&tx.id, Stage::Queued, Stage::Cancelled, None)?;
        }
        self.set_plan_state(id, "cancelled")
    }

    pub fn set_plan_state(&mut self, id: &str, state: &str) -> Result<(), AppError> {
        validate_id(id)?;
        if !["ready", "running", "paused", "finished", "cancelled"].contains(&state) {
            return Err(AppError::new("INVALID_PLAN_STATE", "计划状态无效。"));
        }
        if self
            .connection
            .execute("UPDATE plans SET state=?1 WHERE id=?2", params![state, id])
            .map_err(db_error)?
            != 1
        {
            return Err(AppError::new("PLAN_NOT_FOUND", "更新计划不存在。"));
        }
        Ok(())
    }

    pub fn transition(
        &mut self,
        id: &str,
        expected: Stage,
        next: Stage,
        failure: Option<&str>,
    ) -> Result<UpdateTransaction, AppError> {
        if !expected.allows(next) {
            return Err(AppError::new("INVALID_TRANSITION", "任务阶段转换无效。"));
        }
        let mut value = self.transaction(id)?;
        if value.stage != expected {
            return Err(AppError::new(
                "TRANSACTION_CONFLICT",
                "任务阶段已变化，请重新读取。",
            ));
        }
        if matches!(
            next,
            Stage::Prepared | Stage::Installing | Stage::Recovering
        ) {
            let restore = value
                .restore_point_id
                .as_deref()
                .ok_or_else(|| AppError::new("RESTORE_NOT_READY", "恢复材料尚未准备完成。"))?;
            if self.verify_restore_point(restore)?.transaction_id != id {
                return Err(AppError::new("RESTORE_IDENTITY", "恢复点不属于当前事务。"));
            }
        }
        value.stage = next;
        value.updated_at = now();
        value.failure_code = failure.map(str::to_string);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let count = transaction
            .execute(
                "UPDATE transactions SET stage=?1,body=?2,updated_at=?3 WHERE id=?4 AND stage=?5",
                params![
                    next.key(),
                    encode(&value)?,
                    value.updated_at,
                    id,
                    expected.key()
                ],
            )
            .map_err(db_error)?;
        if count != 1 {
            return Err(AppError::new("TRANSACTION_CONFLICT", "任务阶段已变化。"));
        }
        transaction
            .execute(
                "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,?2,?3)",
                params![id, next.key(), value.updated_at],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(value)
    }

    pub fn begin_restore_point(&self, id: &str) -> Result<PathBuf, AppError> {
        validate_id(id)?;
        let path = self.root.join("restores").join(format!("{id}.partial"));
        if fs::symlink_metadata(&path).is_ok() {
            return Err(AppError::new("RESTORE_EXISTS", "恢复材料目录已存在。"));
        }
        fs::create_dir(&path)
            .map_err(|_| AppError::new("RESTORE_DIRECTORY", "无法创建恢复材料目录。"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .map_err(|_| AppError::new("DATA_PERMISSIONS", "恢复材料权限无法限制。"))?;
        Ok(path)
    }

    pub fn publish_restore_point(
        &mut self,
        transaction_id: &str,
        id: &str,
    ) -> Result<RestorePoint, AppError> {
        validate_id(id)?;
        let mut value = self.transaction(transaction_id)?;
        if value.stage != Stage::BackingUp {
            return Err(AppError::new(
                "INVALID_TRANSITION",
                "当前阶段不能发布恢复点。",
            ));
        }
        let partial = self.root.join("restores").join(format!("{id}.partial"));
        let final_path = self.root.join("restores").join(id);
        if !partial.exists() {
            return Err(AppError::new("RESTORE_MISSING", "恢复材料暂存目录不存在。"));
        }
        files::private_directory(&partial)?;
        if fs::symlink_metadata(&final_path).is_ok() {
            return Err(AppError::new("RESTORE_EXISTS", "恢复点已存在，不能覆盖。"));
        }
        let bytes = read_manifest(&partial.join("manifest.json"))?;
        validate_manifest(&bytes, &value, id)?;
        files::sync_directory(&partial)?;
        fs::rename(&partial, &final_path)
            .map_err(|_| AppError::new("RESTORE_PUBLISH", "恢复材料尚未完整发布。"))?;
        files::sync_directory(&self.root.join("restores"))?;
        let point = RestorePoint {
            id: id.into(),
            transaction_id: transaction_id.into(),
            relative_path: format!("restores/{id}"),
            manifest_sha256: format!("{:x}", Sha256::digest(&bytes)),
            created_at: now(),
        };
        value.stage = Stage::Prepared;
        value.restore_point_id = Some(id.into());
        value.updated_at = now();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO restore_points(id,transaction_id,body)VALUES(?1,?2,?3)",
                params![id, transaction_id, encode(&point)?],
            )
            .map_err(db_error)?;
        if transaction.execute("UPDATE transactions SET stage='prepared',restore_id=?1,body=?2,updated_at=?3 WHERE id=?4 AND stage='backing_up'",params![id,encode(&value)?,value.updated_at,transaction_id]).map_err(db_error)?!=1{return Err(AppError::new("TRANSACTION_CONFLICT","准备阶段已变化；已发布材料保留为孤立恢复点。"));}
        transaction
            .execute(
                "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,'prepared',?2)",
                params![transaction_id, value.updated_at],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(point)
    }

    pub fn restore_point(&self, id: &str) -> Result<RestorePoint, AppError> {
        validate_id(id)?;
        let body: Option<String> = self
            .connection
            .query_row("SELECT body FROM restore_points WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_error)?;
        decode(&body.ok_or_else(|| AppError::new("RESTORE_NOT_FOUND", "恢复点记录不存在。"))?)
    }
    pub fn verify_restore_point(&self, id: &str) -> Result<RestorePoint, AppError> {
        let point = self.restore_point(id)?;
        if point.id != id || point.relative_path != format!("restores/{id}") {
            return Err(AppError::new("RESTORE_PATH", "恢复点引用路径无效。"));
        }
        let directory = self.root.join(&point.relative_path);
        if !directory.exists() {
            return Err(AppError::new("RESTORE_MISSING", "恢复材料目录不存在。"));
        }
        files::private_directory(&directory)?;
        let bytes = read_manifest(&directory.join("manifest.json"))?;
        if format!("{:x}", Sha256::digest(&bytes)) != point.manifest_sha256 {
            return Err(AppError::new(
                "RESTORE_CORRUPT",
                "恢复清单损坏；不能开始修改或恢复。",
            ));
        }
        validate_manifest(&bytes, &self.transaction(&point.transaction_id)?, id)?;
        Ok(point)
    }
    pub fn detach_restore(&mut self, original: &UpdateTransaction) -> Result<(), AppError> {
        if original.stage.active() {
            return Err(AppError::new("RESTORE_PROTECTED", "进行中恢复点不能清理。"));
        }
        let mut value = self.transaction(&original.id)?;
        if value.stage != original.stage || value.restore_point_id != original.restore_point_id {
            return Err(AppError::new(
                "TRANSACTION_CONFLICT",
                "清理期间事务已变化。",
            ));
        }
        let restore = value
            .restore_point_id
            .take()
            .ok_or_else(|| AppError::new("RESTORE_NOT_FOUND", "此事务没有恢复点。"))?;
        value.updated_at = now();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        if transaction.execute("UPDATE transactions SET restore_id=NULL,body=?1,updated_at=?2 WHERE id=?3 AND stage=?4 AND restore_id=?5",params![encode(&value)?,value.updated_at,value.id,value.stage.key(),restore]).map_err(db_error)? != 1 {return Err(AppError::new("TRANSACTION_CONFLICT", "清理期间事务已变化。"));}
        transaction
            .execute("DELETE FROM restore_points WHERE id=?1", [restore])
            .map_err(db_error)?;
        transaction
            .execute(
                "INSERT INTO events(transaction_id,code,created_at)VALUES(?1,'restore_deleted',?2)",
                params![value.id, value.updated_at],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }

    pub fn events(&self, transaction_id: &str) -> Result<Vec<Event>, AppError> {
        validate_id(transaction_id)?;
        let mut statement = self
            .connection
            .prepare("SELECT id,code,created_at FROM events WHERE transaction_id=?1 ORDER BY id")
            .map_err(db_error)?;
        let rows = statement
            .query_map([transaction_id], |r| {
                Ok(Event {
                    id: r.get(0)?,
                    transaction_id: transaction_id.into(),
                    code: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
    }
}
fn read_manifest(path: &Path) -> Result<Vec<u8>, AppError> {
    files::regular_private_file(path)?;
    let file =
        fs::File::open(path).map_err(|_| AppError::new("RESTORE_MISSING", "恢复清单不可读取。"))?;
    let mut bytes = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AppError::new("RESTORE_MISSING", "恢复清单不可读取。"))?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(AppError::new("RESTORE_CORRUPT", "恢复清单超过允许范围。"));
    }
    Ok(bytes)
}
fn validate_manifest(bytes: &[u8], value: &UpdateTransaction, id: &str) -> Result<(), AppError> {
    let header: ManifestHeader = serde_json::from_slice(bytes)
        .map_err(|_| AppError::new("RESTORE_CORRUPT", "恢复清单格式无效。"))?;
    if header.schema_version != 1
        || header.id != id
        || header.transaction_id != value.id
        || header.installation_id != value.installation.id
        || Some(header.original_version.as_str()) != value.installation.version.as_deref()
        || header.target_version != value.target.version
    {
        return Err(AppError::new(
            "RESTORE_IDENTITY",
            "恢复清单与任务或安装实例不匹配。",
        ));
    }
    Ok(())
}
