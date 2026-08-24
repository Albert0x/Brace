use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};

// ---------- 输入诊断 ----------
// 输入法相关的问题（比如 Win10 + 第三方输入法的重复输入）只在最终用户的机器上复现，
// 而 release 包里 DevTools 是关的——console 打了也没人看得见。所以只能落盘成文件，
// 让用户把文件发回来。开关在设置界面里，不能依赖 console 执行命令去开。

const DEBUG_LOG_MAX: u64 = 4_000_000;

fn debug_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到配置目录：{}", e))?;
    Ok(dir.join("input-debug.log"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DebugLogInfo {
    path: String,
    size: u64,
    exists: bool,
}

#[tauri::command]
pub(crate) fn debug_log_info(app: AppHandle) -> Result<DebugLogInfo, String> {
    let path = debug_log_path(&app)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(DebugLogInfo {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
        size,
    })
}

#[tauri::command]
pub(crate) fn append_debug_log(app: AppHandle, lines: Vec<String>) -> Result<(), String> {
    if lines.is_empty() {
        return Ok(());
    }
    let path = debug_log_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }

    // 到了上限就停笔，不做轮转。一次复现用不了几十 KB，涨到 4MB 只说明诊断忘了关，
    // 这时候继续写下去只是在悄悄吃硬盘
    let existing = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if existing > DEBUG_LOG_MAX {
        return Ok(());
    }

    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    // 空文件先写个抬头，说明这文件是什么、里面有什么、怎么关掉
    if existing == 0 {
        let _ = writeln!(
            f,
            "# Brace 输入诊断日志\n\
             # 这个文件记录你在终端里的按键、输入法组合事件和最终发往终端的字符，\n\
             # 用于排查输入法重复输入之类的问题。它包含你输入的全部内容。\n\
             # 关闭：设置 → 关于 → 输入诊断。删除：同一处的「清除日志」。\n"
        );
    }
    for line in lines {
        writeln!(f, "{}", line).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn clear_debug_log(app: AppHandle) -> Result<(), String> {
    let path = debug_log_path(&app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
