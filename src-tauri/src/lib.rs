pub mod agents;
pub mod artifacts;
pub mod backup;
pub mod cli_install;
pub mod cli_launch;
pub mod deploy;
pub mod desktop_apps;
pub mod error;
pub mod panel;
pub mod platform;
pub mod progress;
pub mod releases;
pub mod service;
pub mod storage;
pub mod updater;

#[tauri::command]
fn hide_main_window(app: tauri::AppHandle) -> Result<(), error::AppError> {
    panel::hide(&app)
}

#[tauri::command]
fn get_cli_projects(app: tauri::AppHandle) -> Result<Vec<String>, error::AppError> {
    use tauri::Manager;
    service::with_store(&app.state::<service::AppState>(), |s| {
        cli_launch::projects(&s.root)
    })
}

#[tauri::command]
fn save_cli_project(
    app: tauri::AppHandle,
    path: String,
    previous_path: Option<String>,
) -> Result<Vec<String>, error::AppError> {
    use tauri::Manager;
    service::with_store(&app.state::<service::AppState>(), |s| {
        cli_launch::save_project(&s.root, &path, previous_path.as_deref())
    })
}

#[tauri::command]
async fn open_cli(
    app: tauri::AppHandle,
    instance_id: String,
    path: String,
    previous_path: Option<String>,
    profile: Option<String>,
) -> Result<(), error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<service::AppState>();
        let installation = state
            .installations
            .lock()
            .map_err(|_| error::AppError::new("CLI_INSTANCE", "请重新扫描 CLI。"))?
            .iter()
            .find(|i| i.id == instance_id)
            .cloned()
            .ok_or_else(|| {
                error::AppError::new("CLI_INSTANCE", "CLI 安装实例已失效，请重新扫描。")
            })?;
        let root = service::with_store(&state, |s| Ok(s.root.clone()))?;
        let _operation = service::begin_operation(
            &state,
            &root,
            false,
            Some(&format!("cli:{}", installation.agent_id.key())),
        )?;
        cli_launch::launch_with_profile(&installation, &path, profile.as_deref())?;
        service::with_store(&state, |s| {
            cli_launch::save_project(&s.root, &path, previous_path.as_deref())
        })?;
        Ok(())
    })
    .await
    .map_err(|_| error::AppError::new("CLI_TERMINAL_OPEN", "终端打开操作未完成。"))?
}

#[tauri::command]
async fn install_cli(
    app: tauri::AppHandle,
    agent: agents::AgentId,
    allow_prerelease: bool,
    on_progress: tauri::ipc::Channel<progress::Progress>,
) -> Result<cli_install::Summary, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        progress::with_channel(Some(on_progress), || {
            progress::operation(&format!("cli:{}", agent.key()), || {
                let state = app.state::<service::AppState>();
                let root = service::with_store(&state, |s| Ok(s.root.clone()))?;
                let _op = service::begin_operation(
                    &state,
                    &root,
                    false,
                    Some(&format!("cli:{}", agent.key())),
                )?;
                Ok(cli_install::install(
                    &root,
                    &agents::discovery::DiscoveryContext::current()?,
                    agent,
                    allow_prerelease,
                )?
                .into())
            })
        })
    })
    .await
    .map_err(|_| error::AppError::new("INSTALL_FAILED", "安装未完成，请查看持久记录。"))?
}
#[tauri::command]
async fn cli_install_history(
    app: tauri::AppHandle,
) -> Result<Vec<cli_install::Summary>, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let root = service::with_store(&app.state::<service::AppState>(), |s| Ok(s.root.clone()))?;
        Ok(cli_install::history(&root)?
            .into_iter()
            .map(Into::into)
            .collect())
    })
    .await
    .map_err(|_| error::AppError::new("INSTALL_RECORD", "安装记录读取失败。"))?
}
#[tauri::command]
async fn prepare_app_install(
    app: tauri::AppHandle,
    product: deploy::BundleProduct,
) -> Result<deploy::Summary, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<service::AppState>();
        let root = service::with_store(&state, |s| Ok(s.root.clone()))?;
        let _op = service::begin_operation(
            &state,
            &root,
            product == deploy::BundleProduct::Agentupdate,
            Some(&format!("app:{}", product.key())),
        )?;
        let home = agents::discovery::DiscoveryContext::current()?.home;
        let package = if let Some(p) = product.desktop() {
            desktop_apps::package(p, std::env::consts::ARCH)?
        } else {
            releases::install_package(&root, env!("CARGO_PKG_VERSION"), std::env::consts::ARCH)?
        };
        Ok(deploy::prepare(&root, &home, product, &package, std::env::consts::ARCH)?.into())
    })
    .await
    .map_err(|_| error::AppError::new("APP_PREPARE", "应用安装准备未完成，旧应用保留。"))?
}
#[tauri::command]
async fn app_install_history(
    app: tauri::AppHandle,
) -> Result<Vec<deploy::Summary>, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let root = service::with_store(&app.state::<service::AppState>(), |s| Ok(s.root.clone()))?;
        Ok(deploy::history(&root)?
            .into_iter()
            .map(Into::into)
            .collect())
    })
    .await
    .map_err(|_| error::AppError::new("APP_RECORD", "应用安装记录读取失败。"))?
}
// Both direct installs and resumable prepared records keep the process lock throughout.
fn complete_app_install(
    app: &tauri::AppHandle,
    state: &service::AppState,
    root: &std::path::Path,
    home: &std::path::Path,
    record: deploy::Record,
) -> Result<deploy::Summary, error::AppError> {
    if record.product == deploy::BundleProduct::Agentupdate {
        deploy::validate_destination(home, record.product, &record.destination)?;
        if deploy::destination(home, record.product)? != record.destination {
            return Err(error::AppError::new(
                "APP_SELF_PATH",
                "当前运行应用与更新目标不同。",
            ));
        }
        progress::phase(progress::Phase::Installing);
        deploy::launch_helper(root, &record.id, false)?;
        state
            .restarting
            .store(true, std::sync::atomic::Ordering::SeqCst);
        progress::phase(progress::Phase::Restarting);
        let handle = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            handle.exit(0);
            std::thread::sleep(std::time::Duration::from_secs(8));
            std::process::exit(0);
        });
        let mut summary: deploy::Summary = record.into();
        summary.state = "restarting".into();
        Ok(summary)
    } else {
        Ok(deploy::install_locked(root, home, &record.id, std::env::consts::ARCH)?.into())
    }
}
#[tauri::command]
async fn install_app(
    app: tauri::AppHandle,
    product: deploy::BundleProduct,
    on_progress: tauri::ipc::Channel<progress::Progress>,
) -> Result<deploy::Summary, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        progress::with_channel(Some(on_progress), || {
            progress::operation(&format!("app:{}", product.key()), || {
                let state = app.state::<service::AppState>();
                let root = service::with_store(&state, |s| Ok(s.root.clone()))?;
                let _op = service::begin_operation(
                    &state,
                    &root,
                    product == deploy::BundleProduct::Agentupdate,
                    Some(&format!("app:{}", product.key())),
                )?;
                let home = agents::discovery::DiscoveryContext::current()?.home;
                let package = if let Some(p) = product.desktop() {
                    desktop_apps::package(p, std::env::consts::ARCH)?
                } else {
                    releases::install_package(
                        &root,
                        env!("CARGO_PKG_VERSION"),
                        std::env::consts::ARCH,
                    )?
                };
                if product.desktop().is_some() {
                    let existing = desktop_apps::discover(&home);
                    for (_, installed) in existing
                        .iter()
                        .filter(|(p, _)| Some(*p) == product.desktop())
                    {
                        let ordering = installed
                            .version
                            .as_deref()
                            .and_then(|v| desktop_apps::compare(v, &package.version));
                        if ordering != Some(std::cmp::Ordering::Less) {
                            return Err(error::AppError::new(
                                "APP_NOT_NEWER",
                                "没有可确认的更新版本，请重新检测。",
                            ));
                        }
                    }
                }
                let record =
                    deploy::prepare(&root, &home, product, &package, std::env::consts::ARCH)?;
                complete_app_install(&app, &state, &root, &home, record)
            })
        })
    })
    .await
    .map_err(|_| error::AppError::new("APP_INSTALL", "应用安装未完成，请查看持久记录。"))?
}
#[tauri::command]
async fn finish_app_install(
    app: tauri::AppHandle,
    id: String,
    on_progress: tauri::ipc::Channel<progress::Progress>,
) -> Result<deploy::Summary, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        progress::with_channel(Some(on_progress), || {
            let state = app.state::<service::AppState>();
            let root = service::with_store(&state, |s| Ok(s.root.clone()))?;
            let home = agents::discovery::DiscoveryContext::current()?.home;
            let record = deploy::history(&root)?
                .into_iter()
                .find(|r| r.id == id)
                .ok_or_else(|| error::AppError::new("APP_INSTALL_ID", "安装记录不存在。"))?;
            let product = record.product;
            let _op = service::begin_operation(
                &state,
                &root,
                product == deploy::BundleProduct::Agentupdate,
                Some(&format!("app:{}", product.key())),
            )?;
            progress::operation(&format!("app:{}", record.product.key()), || {
                complete_app_install(&app, &state, &root, &home, record)
            })
        })
    })
    .await
    .map_err(|_| error::AppError::new("APP_INSTALL", "应用安装未完成，请查看持久记录。"))?
}
#[tauri::command]
async fn check_desktop_apps(
    product: Option<desktop_apps::Product>,
) -> Result<Vec<desktop_apps::AppCheck>, error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| error::AppError::new("HOME_UNAVAILABLE", "无法确认用户目录。"))?;
        Ok(desktop_apps::check_selected(
            &home,
            std::env::consts::ARCH,
            product,
        ))
    })
    .await
    .map_err(|_| error::AppError::new("APP_CHECK_FAILED", "桌面应用检查未完成。"))?
}
#[tauri::command]
async fn open_desktop_app_channel(
    product: desktop_apps::Product,
    download: bool,
) -> Result<(), error::AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        desktop_apps::open(product, download, std::env::consts::ARCH)
    })
    .await
    .map_err(|_| error::AppError::new("APP_OPEN_FAILED", "无法打开官方更新入口。"))?
}

#[tauri::command]
async fn check_app_release(
    app: tauri::AppHandle,
    manual: bool,
) -> Result<releases::CheckResult, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let root = service::with_store(&app.state::<service::AppState>(), |store| {
            Ok(store.root.clone())
        })?;
        releases::check(
            &root,
            manual,
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
        )
    })
    .await
    .map_err(|_| error::AppError::new("RELEASE_CHECK_FAILED", "应用版本检查未完成。"))?
}
#[tauri::command]
async fn open_app_release(app: tauri::AppHandle, asset: bool) -> Result<(), error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let root = service::with_store(&app.state::<service::AppState>(), |store| {
            Ok(store.root.clone())
        })?;
        releases::open_link(
            &root,
            asset,
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
        )
    })
    .await
    .map_err(|_| error::AppError::new("RELEASE_OPEN", "无法打开发布入口。"))?
}

#[tauri::command]
async fn export_transaction(
    app: tauri::AppHandle,
    transaction_id: String,
    kind: storage::history::ExportKind,
    confirmed_sensitive: bool,
) -> Result<storage::history::ExportResult, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<service::AppState>();
        let _operation = state
            .operation
            .try_write()
            .map_err(|_| error::AppError::new("OPERATION_BUSY", "请等待当前操作结束后导出。"))?;
        let root = service::with_store(&state, |store| Ok(store.root.clone()))?;
        storage::history::export(
            &storage::Store::open(&root)?,
            &transaction_id,
            kind,
            confirmed_sensitive,
        )
    })
    .await
    .map_err(|_| error::AppError::new("EXPORT_FAILED", "导出未完成。"))?
}
#[tauri::command]
async fn reveal_export(app: tauri::AppHandle, export_id: String) -> Result<(), error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        service::with_store(&app.state::<service::AppState>(), |store| {
            storage::history::reveal_export(store, &export_id)
        })
    })
    .await
    .map_err(|_| error::AppError::new("EXPORT_OPEN", "导出目录无法打开。"))?
}

#[tauri::command]
async fn inspect_interrupted_tasks(
    app: tauri::AppHandle,
) -> Result<updater::recovery::RecoveryReport, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<service::AppState>();
        let _operation = match state.operation.try_write() {
            Ok(lock) => lock,
            Err(_) => {
                return Ok(updater::recovery::RecoveryReport {
                    busy: true,
                    items: vec![],
                })
            }
        };
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| error::AppError::new("HOME_UNAVAILABLE", "无法确认用户目录。"))?;
        service::with_store(&state, |store| updater::recovery::reconcile(store, &home))
    })
    .await
    .map_err(|_| {
        error::AppError::new(
            "RECOVERY_CHECK_FAILED",
            "未完成任务检查失败，请保留诊断记录。",
        )
    })?
}

#[tauri::command]
async fn execute_plan(
    app: tauri::AppHandle,
    plan_id: String,
    continue_after_failure: bool,
    on_progress: tauri::ipc::Channel<progress::Progress>,
) -> Result<storage::model::UpdatePlan, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        progress::with_channel(Some(on_progress), || {
            service::execute_plan(
                &app.state::<service::AppState>(),
                &plan_id,
                continue_after_failure,
            )
        })
    })
    .await
    .map_err(|_| error::AppError::new("QUEUE_INTERRUPTED", "批量任务未完成，请查看持久化记录。"))?
}
#[tauri::command]
async fn cancel_plan(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<storage::model::UpdatePlan, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        service::with_store(&app.state::<service::AppState>(), |store| {
            store.request_cancel(&plan_id)
        })
    })
    .await
    .map_err(|_| error::AppError::new("CANCEL_FAILED", "取消请求未确认，请查看任务记录。"))?
}

#[tauri::command]
async fn execute_update(
    app: tauri::AppHandle,
    transaction_id: String,
    on_progress: tauri::ipc::Channel<progress::Progress>,
) -> Result<storage::model::UpdateTransaction, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        progress::with_channel(Some(on_progress), || {
            service::execute(&app.state::<service::AppState>(), &transaction_id)
        })
    })
    .await
    .map_err(|_| error::AppError::new("UPDATE_INTERRUPTED", "更新工作未完成，请查看事务记录。"))?
}

#[tauri::command]
async fn create_update_plan(
    app: tauri::AppHandle,
    requests: Vec<service::PlanRequest>,
) -> Result<storage::model::UpdatePlan, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        service::build_plan(&app.state::<service::AppState>(), requests)
    })
    .await
    .map_err(|_| error::AppError::new("PLAN_FAILED", "更新计划未完成。"))?
}

#[tauri::command]
async fn get_plan_history(
    app: tauri::AppHandle,
) -> Result<Vec<storage::model::UpdatePlan>, error::AppError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        service::with_store(&app.state::<service::AppState>(), |store| store.plans())
    })
    .await
    .map_err(|_| error::AppError::new("HISTORY_FAILED", "历史记录暂不可用。"))?
}

#[tauri::command]
fn get_app_overview() -> Result<service::AppOverview, error::AppError> {
    service::overview()
}

#[tauri::command]
fn get_agent_support(agent_id: agents::AgentId) -> agents::AgentSupport {
    agents::support(agent_id)
}

#[tauri::command]
async fn discover_agents(
    extra_paths: Vec<std::path::PathBuf>,
    state: tauri::State<'_, service::AppState>,
) -> Result<agents::discovery::DiscoveryReport, error::AppError> {
    let report = tauri::async_runtime::spawn_blocking(move || {
        agents::discovery::discover(
            &agents::discovery::DiscoveryContext::current()?,
            &extra_paths,
        )
    })
    .await
    .map_err(|_| error::AppError::new("DISCOVERY_FAILED", "工具扫描未完成。"))??;
    *state
        .installations
        .lock()
        .map_err(|_| error::AppError::new("STATE_UNAVAILABLE", "本地状态暂不可用。"))? =
        report.instances.clone();
    Ok(report)
}

#[tauri::command]
async fn list_agent_versions(
    agent_id: agents::AgentId,
    include_prerelease: bool,
    instance_id: Option<String>,
    state: tauri::State<'_, service::AppState>,
) -> Result<agents::versions::VersionCheck, error::AppError> {
    let instance = if let Some(id) = instance_id {
        Some(
            state
                .installations
                .lock()
                .map_err(|_| error::AppError::new("STATE_UNAVAILABLE", "实例信息暂不可用。"))?
                .iter()
                .find(|i| i.id == id && i.agent_id == agent_id)
                .cloned()
                .ok_or_else(|| {
                    error::AppError::new("INSTANCE_UNAVAILABLE", "请重新扫描安装实例。")
                })?,
        )
    } else {
        None
    };
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = if let Some(i) = instance.as_ref().filter(|i| updater::native::handles(i)) {
            updater::native::catalog(i)?
        } else {
            agents::versions::fetch(agent_id, include_prerelease)?
        };
        let current = instance
            .as_ref()
            .filter(|i| i.version_verified)
            .and_then(|i| i.version.as_deref());
        let state = agents::versions::update_state(current, &catalog.latest_stable);
        Ok(agents::versions::VersionCheck { catalog, state })
    })
    .await
    .map_err(|_| error::AppError::new("VERSION_QUERY_FAILED", "版本查询未完成。"))?
}

#[tauri::command]
async fn select_agent_version(
    agent_id: agents::AgentId,
    target: String,
    allow_prerelease: bool,
) -> Result<agents::versions::AvailableVersion, error::AppError> {
    if target.len() > 128 {
        return Err(error::AppError::new("INVALID_VERSION", "版本标识过长。"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = agents::versions::fetch(agent_id, allow_prerelease)?;
        agents::versions::select(&catalog, &target, allow_prerelease)
    })
    .await
    .map_err(|_| error::AppError::new("VERSION_QUERY_FAILED", "目标版本核验未完成。"))?
}

#[cfg(feature = "desktop-validation")]
mod validation {
    use serde::{Deserialize, Serialize};

    #[tauri::command]
    pub async fn probe_panel(
        app: tauri::AppHandle,
        action: String,
    ) -> Result<serde_json::Value, String> {
        use tauri::Manager;
        tauri::async_runtime::spawn_blocking(move || {
            let window = app
                .get_webview_window("main")
                .ok_or("Main window missing")?;
            match action.as_str() {
                "snapshot" => {}
                "tray_click" => super::panel::handle_click(
                    &app,
                    tauri::tray::MouseButton::Left,
                    tauri::tray::MouseButtonState::Up,
                ),
                "menu_open" => super::panel::menu_action(&app, "panel-open"),
                "reopen" => super::panel::show(&app).map_err(|e| e.to_string())?,
                "close" => window.close().map_err(|e| e.to_string())?,
                "minimize" => window.minimize().map_err(|e| e.to_string())?,
                "quit_busy" => {
                    let state = app.state::<super::service::AppState>();
                    let _operation = state.operation.write().map_err(|e| e.to_string())?;
                    app.exit(0);
                    std::thread::sleep(std::time::Duration::from_millis(800));
                }
                _ => return Err("Unknown panel probe".into()),
            }
            Ok(serde_json::json!({
                "visible": window.is_visible().map_err(|e| e.to_string())?,
                "minimized": window.is_minimized().map_err(|e| e.to_string())?,
                "trayPresent": app.tray_by_id("agentupdate-panel").is_some(),
            }))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    #[tauri::command]
    pub fn finish_panel_validation(
        app: tauri::AppHandle,
        checks: std::collections::BTreeMap<String, bool>,
    ) -> Result<(), String> {
        let destination = std::env::var_os("AGENTUPDATE_VALIDATION_RESULT")
            .ok_or("Missing validation destination")?;
        std::fs::write(
            destination,
            serde_json::to_vec_pretty(
                &serde_json::json!({"checks": checks, "architecture": std::env::consts::ARCH}),
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        app.exit(0);
        Ok(())
    }

    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct Checks {
        pub navigation_works: bool,
        pub per_tool_detection: bool,
        pub desktop_upgrade_visibility: bool,
        pub domain_call: bool,
        pub invalid_agent_denied: bool,
        pub restricted_command_denied: bool,
        pub arbitrary_command_denied: bool,
        pub arbitrary_file_denied: bool,
        pub shell_plugin_denied: bool,
        pub filesystem_plugin_denied: bool,
        pub desktop_products_checked: bool,
        pub invalid_desktop_product_denied: bool,
        pub desktop_rendered: bool,
        pub rendered: bool,
        pub install_commands_ready: bool,
        pub invalid_install_agent_denied: bool,
        pub invalid_install_product_denied: bool,
    }
    #[derive(Deserialize, Serialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct WorkflowChecks {
        pub navigation_works: bool,
        pub rendered: bool,
        pub selected_both: bool,
        pub no_install_confirmation: bool,
        pub progress_observed: bool,
        pub updated_buttons_hidden: bool,
        pub both_updated: bool,
        pub rollback_removed: bool,
        pub private_log_exported: bool,
        pub backup_protected: bool,
        pub release_prompt: bool,
        pub release_badge: bool,
        pub release_nav_fits: bool,
        pub release_label: bool,
        pub release_banner: bool,
        pub release_banner_navigates: bool,
        pub update_reminder_persists: bool,
        pub desktop_direct_attempt: bool,
        pub desktop_progress: bool,
        pub desktop_unsigned_fixture_rejected: bool,
        pub desktop_original_preserved: bool,
    }
    #[tauri::command]
    pub fn finish_workflow_validation(
        app: tauri::AppHandle,
        checks: WorkflowChecks,
    ) -> Result<(), String> {
        use tauri::Manager;
        let destination = std::env::var_os("AGENTUPDATE_VALIDATION_RESULT")
            .ok_or("Missing validation destination")?;
        let plans = super::service::with_store(&app.state::<super::service::AppState>(), |store| {
            store.plans()
        })
        .map_err(|e| e.message)?;
        let transactions:Vec<_>=plans.into_iter().flat_map(|p|p.transactions).map(|tx|serde_json::json!({"agent":tx.installation.agent_id,"original":tx.installation.version,"target":tx.target.version,"stage":tx.stage,"restorePoint":tx.restore_point_id.is_some()})).collect();
        let value = serde_json::json!({"architecture":std::env::consts::ARCH,"checks":checks,"transactions":transactions});
        std::fs::write(
            destination,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        app.exit(0);
        Ok(())
    }
    #[tauri::command]
    pub fn finish_cli_launch_validation(
        app: tauri::AppHandle,
        checks: std::collections::BTreeMap<String, bool>,
    ) -> Result<(), String> {
        let destination = std::env::var_os("AGENTUPDATE_VALIDATION_RESULT")
            .ok_or("Missing validation destination")?;
        let value = serde_json::json!({"architecture":std::env::consts::ARCH,"checks":checks});
        std::fs::write(
            destination,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        app.exit(0);
        Ok(())
    }
    #[tauri::command]
    pub fn restricted_probe() -> bool {
        true
    }

    #[tauri::command]
    pub fn finish_desktop_validation(app: tauri::AppHandle, checks: Checks) -> Result<(), String> {
        // Developer-only binary and capability. Destination is set by the runner, never by IPC.
        let destination = std::env::var_os("AGENTUPDATE_VALIDATION_RESULT")
            .ok_or("Missing validation destination")?;
        let value = serde_json::json!({ "overview": super::service::overview().map_err(|e| e.message)?, "checks": checks });
        std::fs::write(
            destination,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        app.exit(0);
        Ok(())
    }
}

pub fn run() {
    let builder = tauri::Builder::default().manage(service::AppState::default());
    #[cfg(not(feature = "desktop-validation"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_app_overview,
        hide_main_window,
        check_app_release,
        check_desktop_apps,
        get_cli_projects,
        save_cli_project,
        open_cli,
        install_cli,
        cli_install_history,
        prepare_app_install,
        install_app,
        app_install_history,
        finish_app_install,
        open_desktop_app_channel,
        open_app_release,
        get_agent_support,
        discover_agents,
        list_agent_versions,
        select_agent_version,
        create_update_plan,
        get_plan_history,
        export_transaction,
        reveal_export,
        inspect_interrupted_tasks,
        execute_update,
        execute_plan,
        cancel_plan,
    ]);
    #[cfg(feature = "desktop-validation")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_app_overview,
        hide_main_window,
        check_app_release,
        check_desktop_apps,
        get_cli_projects,
        save_cli_project,
        open_cli,
        install_cli,
        cli_install_history,
        prepare_app_install,
        install_app,
        app_install_history,
        finish_app_install,
        open_desktop_app_channel,
        open_app_release,
        get_agent_support,
        discover_agents,
        list_agent_versions,
        select_agent_version,
        create_update_plan,
        get_plan_history,
        export_transaction,
        reveal_export,
        inspect_interrupted_tasks,
        execute_update,
        execute_plan,
        cancel_plan,
        validation::restricted_probe,
        validation::probe_panel,
        validation::finish_panel_validation,
        validation::finish_cli_launch_validation,
        validation::finish_desktop_validation,
        validation::finish_workflow_validation
    ]);
    builder
        .setup(|app| {
            panel::setup(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Could not build AgentUpdate")
        .run(panel::on_run_event);
}
