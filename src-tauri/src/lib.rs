// 后端按领域分模块。这里只留装配：注册状态、挂命令、退出时收尸。
//
// 拆分前是一个 2800 行的 lib.rs，PTY、文件系统、git、密钥加密、用量统计、
// shell 探测七个互不相干的领域挤在一起，加上 macOS 的 cfg 分支之后更难读了。
mod diagnostics;
mod fs_ops;
mod git;
mod preview;
mod profiles;
mod pty;
mod shells;
mod ssh;
mod system;
mod usage;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(pty::PtyManager::default())
        .manage(fs_ops::FsWatcher::default())
        .setup(|_app| {
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::apply_acrylic;
                if let Some(window) = _app.get_webview_window("main") {
                    // 只有 Win11 才上 acrylic；Win10 的 acrylic 边缘有黑边、拖动卡，
                    // 退回普通背景层（窗口正常，只是少了那层毛玻璃）
                    if system::is_win11() {
                        let _ = apply_acrylic(&window, Some((18, 18, 18, 160)));
                    }
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pty::pty_create,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_close,
            fs_ops::list_dir,
            fs_ops::home_dir,
            fs_ops::watch_dirs,
            fs_ops::create_entry,
            fs_ops::rename_entry,
            fs_ops::delete_entry,
            shells::detect_shells,
            ssh::load_ssh_sessions,
            ssh::save_ssh_sessions,
            ssh::ssh_client_path,
            ssh::ssh_launch,
            usage::usage_stats,
            usage::statusline_status,
            usage::configure_statusline,
            git::git_status,
            git::git_commit,
            git::git_diff,
            preview::read_file,
            preview::write_file,
            system::os_version,
            system::system_proxy,
            system::save_bg_image,
            system::load_bg_image,
            profiles::load_profiles,
            profiles::save_profiles,
            diagnostics::debug_log_info,
            diagnostics::append_debug_log,
            diagnostics::clear_debug_log
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Exit 是事件循环退出前的最后时机，覆盖所有退出路径（关窗口、托盘退出、
            // 更新后 relaunch），在这里统一回收 PTY 进程树
            if let tauri::RunEvent::Exit = event {
                pty::kill_all_sessions(&app.state::<pty::PtyManager>());
            }
        });
}
