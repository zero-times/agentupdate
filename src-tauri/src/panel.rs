use crate::{error::AppError, service::AppState};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

const TRAY_ID: &str = "agentupdate-panel";

fn tray_image() -> tauri::image::Image<'static> {
    // A monochrome arrow with a top bar. macOS tints template images
    // automatically for light/dark menu bars; the surrounding pixels stay clear.
    let mut rgba = vec![0; 22 * 22 * 4];
    for y in 0..22usize {
        for x in 0..22usize {
            let arrow = (8..=15).contains(&y) && (x == 18 - y || x == y + 2);
            let stem = (6..=15).contains(&y) && (10..=11).contains(&x);
            let top = (4..=5).contains(&y) && (4..=17).contains(&x);
            let base = (18..=19).contains(&y) && (5..=16).contains(&x);
            if arrow || stem || top || base {
                rgba[(y * 22 + x) * 4 + 3] = 255;
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 22, 22)
}

pub fn show(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    Ok(())
}

pub fn hide(app: &AppHandle) -> Result<(), AppError> {
    if app.tray_by_id(TRAY_ID).is_none() {
        return Err(AppError::new(
            "PANEL_TRAY",
            "菜单栏图标尚未就绪，窗口保持打开。",
        ));
    }
    app.get_webview_window("main")
        .ok_or_else(|| AppError::new("PANEL_WINDOW", "工作台窗口不可用。"))?
        .hide()
        .map_err(|_| AppError::new("PANEL_HIDE", "无法收起工作台，请重试。"))
}

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "panel-open", "打开 AgentUpdate", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "panel-hide", "收起到菜单栏", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "panel-quit", "退出 AgentUpdate", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &hide_item, &separator, &quit])?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(tray_image())
        .icon_as_template(true)
        .tooltip("AgentUpdate")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                handle_click(tray.app_handle(), button, button_state);
            }
        })
        .on_menu_event(|app, event| menu_action(app, event.id.as_ref()))
        .build(app)?;
    Ok(())
}

pub fn handle_click(app: &AppHandle, button: MouseButton, state: MouseButtonState) {
    if button == MouseButton::Left && state == MouseButtonState::Up {
        let _ = show(app);
    }
}

pub fn menu_action(app: &AppHandle, id: &str) {
    match id {
        "panel-open" => {
            let _ = show(app);
        }
        "panel-hide" => {
            let _ = hide(app);
        }
        "panel-quit" => app.exit(0),
        _ => {}
    }
}

pub fn on_run_event(app: &AppHandle, event: tauri::RunEvent) {
    match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            // Keep the WebView and any installation task alive when closing.
            api.prevent_close();
            let _ = hide(app);
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            let _ = show(app);
        }
        tauri::RunEvent::ExitRequested { api, .. } => {
            let state = app.state::<AppState>();
            // The self-update helper must be allowed to stop this process.
            if !state.restarting.load(std::sync::atomic::Ordering::SeqCst)
                && state.operation.try_write().is_err()
            {
                api.prevent_exit();
                let _ = show(app);
                let _ = app.emit("panel-exit-blocked", ());
            }
        }
        _ => {}
    }
}
