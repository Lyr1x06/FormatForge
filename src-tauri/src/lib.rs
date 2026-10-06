mod commands;
mod convert;
mod docx;
mod engine;
mod formats;
mod job;
mod paths;
mod pdf;
mod registry;
mod scan;
mod table;

/// 端到端测试的管线入口。不属于产品代码，仅供 `tests/` 使用。
#[doc(hidden)]
pub mod testkit;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_os::init())
        .setup(|app| {
            let script = commands::worker_script_for_setup(app.handle())?;
            app.manage(commands::AppState::new(script));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::probe_engines,
            commands::open_url,
            commands::list_formats,
            commands::scan_inputs,
            commands::start_batch,
            commands::cancel_batch,
            commands::clear_engine_failure,
        ])
        .on_window_event(|window, event| {
            // 关窗即收工：把常驻的 Office worker 干净地关掉，
            // 否则 WINWORD.EXE 会留在后台
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.try_state::<commands::AppState>() {
                    if let Ok(mut pool) = state.office_pool.lock() {
                        pool.shutdown_all();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("启动 Format Forge 失败");
}
