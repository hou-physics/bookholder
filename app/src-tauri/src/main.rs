#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use bookholder_core::{billing, ingest, pricing, store, watcher};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager,
};

fn main() {
    let conn = store::open_db(&store::default_db_path()).expect("open db");
    let _ = pricing::seed_snapshot(&conn);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(commands::Db(Mutex::new(conn)))
        .invoke_handler(tauri::generate_handler![
            commands::float_data,
            commands::overview,
            commands::projects_list,
            commands::project_sessions,
            commands::session_events,
            commands::settings_status,
            commands::refresh_prices,
            commands::run_backfill,
            commands::export_report,
            commands::set_billing_override,
            commands::open_dashboard,
        ])
        .setup(|app| {
            // 托盘
            let show = MenuItem::with_id(app, "show", "打开面板", true, None::<&str>)?;
            let float = MenuItem::with_id(app, "float", "显示/隐藏悬浮窗", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &float, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "float" => {
                        if let Some(w) = app.get_webview_window("float") {
                            if w.is_visible().unwrap_or(false) { let _ = w.hide(); } else { let _ = w.show(); }
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // 后台采集线程：初扫 + 监视 + 每日价格刷新
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let db_path = store::default_db_path();
                let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                let Ok(conn) = store::open_db(&db_path) else { return };
                let mode = billing::effective_mode(&conn, &home);
                let st = ingest::scan_all(&conn, &store::claude_projects_dir(), &mode);
                let _ = pricing::reprice_null_costs(&conn);
                let _ = handle.emit("usage-updated", &st);
                maybe_refresh_prices(&conn);

                let handle2 = handle.clone();
                let db_path2 = db_path.clone();
                let guard = watcher::watch_projects(&store::claude_projects_dir(), move || {
                    let Ok(c) = store::open_db(&db_path2) else { return };
                    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
                    let mode = billing::effective_mode(&c, &home);
                    let st = ingest::scan_all(&c, &store::claude_projects_dir(), &mode);
                    if st.added > 0 {
                        let _ = pricing::reprice_null_costs(&c);
                        let _ = handle2.emit("usage-updated", &st);
                    }
                });
                match guard {
                    Ok(_g) => loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                        if let Ok(c) = store::open_db(&db_path) {
                            maybe_refresh_prices(&c);
                        }
                    },
                    Err(e) => eprintln!("watcher failed: {e}"),
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口点关闭 = 隐藏，应用常驻
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running bookholder");
}

fn maybe_refresh_prices(conn: &rusqlite::Connection) {
    let now = chrono::Utc::now();
    let stale = store::meta_get(conn, "prices_last_fetch")
        .and_then(|s| chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S").ok())
        .map(|t| (now.naive_utc() - t).num_hours() >= 24)
        .unwrap_or(true);
    if stale {
        let ts = now.format("%Y-%m-%d %H:%M:%S").to_string();
        if let Err(e) = pricing::refresh_from_network(conn, &ts) {
            let _ = store::meta_set(conn, "prices_last_status", &format!("error: {e}"));
        }
    }
}
