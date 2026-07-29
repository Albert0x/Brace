use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

// 单个终端会话：持有写入端与主控端
struct PtySession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    pid: Option<u32>,
}

#[derive(Default)]
struct PtyManager {
    sessions: Mutex<HashMap<String, PtySession>>,
}

#[derive(Clone, Serialize)]
struct PtyOutput {
    id: String,
    data: String,
}

// 各 shell 注入自己的 prompt，用 OSC 9;9 上报 cwd（供文件树联动）。结尾只用 \r。
// PowerShell 系：function prompt 里拼 OSC + 可见提示符
const POWERSHELL_INJECT: &str = "function prompt { $p=(Get-Location).Path; $e=[char]27; \"$e]9;9;$p$e\\PS $p> \" }; clear\r";
// CMD：PROMPT 里 $E=ESC、$P=当前路径、$G=>；先 cls 再设，避免回显那行注入命令
const CMD_INJECT: &str = "cls & prompt $E]9;9;$P$E\\$P$G \r";
// Git Bash：PROMPT_COMMAND 每次提示符前 printf 出 OSC 9;9；pwd -W 取 Windows 路径喂文件树
const BASH_INJECT: &str =
    "export PROMPT_COMMAND='printf \"\\033]9;9;%s\\033\\\\\" \"$(pwd -W 2>/dev/null || pwd)\"'\r";

// 新建一个终端会话。shell_path 为空回退 powershell.exe；shell_type 决定是否注入 cwd 上报。
#[tauri::command]
fn pty_create(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    id: String,
    rows: u16,
    cols: u16,
    cwd: String,
    shell_path: String,
    shell_type: String,
) -> Result<(), String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;

    let exe = if shell_path.trim().is_empty() {
        "powershell.exe".to_string()
    } else {
        shell_path
    };
    let mut cmd = CommandBuilder::new(&exe);
    let start_dir = if !cwd.trim().is_empty() {
        cwd
    } else {
        std::env::var("USERPROFILE").unwrap_or_default()
    };
    if !start_dir.is_empty() {
        cmd.cwd(start_dir);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let pid = child.process_id();

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;

    // 按 shell 类型注入对应的 cwd 上报 prompt
    let inject = match shell_type.as_str() {
        "powershell" => POWERSHELL_INJECT,
        "cmd" => CMD_INJECT,
        "bash" => BASH_INJECT,
        _ => "",
    };
    if !inject.is_empty() {
        let _ = writer.write_all(inject.as_bytes());
        let _ = writer.flush();
    }

    let app_handle = app.clone();
    let sid = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_handle.emit(
                        "pty-output",
                        PtyOutput {
                            id: sid.clone(),
                            data,
                        },
                    );
                }
                Err(_) => break,
            }
        }
        let _ = app_handle.emit("pty-exit", sid.clone());
    });

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    manager
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id, PtySession { writer, master: pair.master, pid });
    Ok(())
}

#[tauri::command]
fn pty_write(manager: State<'_, PtyManager>, id: String, data: String) -> Result<(), String> {
    let mut sessions = manager.sessions.lock().map_err(|e| e.to_string())?;
    if let Some(s) = sessions.get_mut(&id) {
        s.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
        s.writer.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn pty_resize(
    manager: State<'_, PtyManager>,
    id: String,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let sessions = manager.sessions.lock().map_err(|e| e.to_string())?;
    if let Some(s) = sessions.get(&id) {
        s.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn pty_close(manager: State<'_, PtyManager>, id: String) -> Result<(), String> {
    manager
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id);
    Ok(())
}

// ---------- 文件系统 ----------

#[derive(Serialize)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
}

#[tauri::command]
fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())?.flatten() {
        let p = entry.path();
        result.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: p.to_string_lossy().to_string(),
            is_dir: p.is_dir(),
        });
    }
    result.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(result)
}

#[tauri::command]
fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "C:\\".to_string())
}

// ---------- Shell 检测 ----------

#[derive(Serialize)]
struct ShellInfo {
    id: String,
    name: String,
    path: String,
    shell_type: String, // powershell | cmd | bash
}

// 在 PATH 中查找可执行文件
fn which(exe: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let full = dir.join(exe);
        if full.is_file() {
            return Some(full.to_string_lossy().to_string());
        }
    }
    None
}

// 检测系统里可用的 shell
#[tauri::command]
fn detect_shells() -> Vec<ShellInfo> {
    let mut shells = Vec::new();

    if let Some(p) = which("powershell.exe") {
        shells.push(ShellInfo {
            id: "powershell".into(),
            name: "Windows PowerShell".into(),
            path: p,
            shell_type: "powershell".into(),
        });
    }
    if let Some(p) = which("pwsh.exe") {
        shells.push(ShellInfo {
            id: "pwsh".into(),
            name: "PowerShell 7".into(),
            path: p,
            shell_type: "powershell".into(),
        });
    }
    if let Some(p) = which("cmd.exe") {
        shells.push(ShellInfo {
            id: "cmd".into(),
            name: "Command Prompt".into(),
            path: p,
            shell_type: "cmd".into(),
        });
    }
    // Git Bash：先从 git.exe 反推安装根（<root>\cmd\git.exe → <root>\bin\bash.exe），
    // 不管 Git 装哪都能找到；找不到再退回标准路径
    let mut bash_path: Option<String> = None;
    if let Some(git) = which("git.exe") {
        if let Some(root) = std::path::Path::new(&git)
            .parent()
            .and_then(|p| p.parent())
        {
            let b = root.join("bin").join("bash.exe");
            if b.is_file() {
                bash_path = Some(b.to_string_lossy().to_string());
            }
        }
    }
    if bash_path.is_none() {
        for cand in [
            "C:\\Program Files\\Git\\bin\\bash.exe",
            "C:\\Program Files (x86)\\Git\\bin\\bash.exe",
        ] {
            if std::path::Path::new(cand).is_file() {
                bash_path = Some(cand.to_string());
                break;
            }
        }
    }
    if let Some(bp) = bash_path {
        shells.push(ShellInfo {
            id: "gitbash".into(),
            name: "Git Bash".into(),
            path: bp,
            shell_type: "bash".into(),
        });
    }

    shells
}

// ---------- Claude 用量统计 ----------
// 数据源：~/.claude/statusline-cache.json，由 HyperTerminal 的 statusLine 采集脚本写入
// （脚本接住 Claude Code 通过 statusLine stdin 喂的官方运行时数据）。
// 这里只负责：① 判断当前标签是否真在跑 claude；② 读缓存把官方 context/5h/7d 吐给前端。

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct UsageStats {
    model: String,
    context_pct: f64,        // 官方上下文占用 %
    five_hour_pct: f64,      // 官方 5h 额度用量 %
    five_hour_reset_ms: i64, // 5h 重置时间（epoch ms，0=无）
    seven_day_pct: f64,      // 官方 7d 额度用量 %
    seven_day_reset_ms: i64, // 7d 重置时间（epoch ms，0=无）
    cache_age_sec: i64,      // 缓存数据距今秒数
    running_claude: bool,    // 当前标签终端是否真在跑 claude
    has_rate_limits: bool,   // 缓存里有没有 5h/7d 额度（发过消息后才有）
    has_data: bool,          // 缓存文件是否存在可读
}

// 判断给定 shell pid 的后代进程里是否有 claude 在跑
fn claude_running_under(root: u32) -> bool {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let procs = sys.processes();
    let mut stack = vec![Pid::from_u32(root)];
    let mut seen = std::collections::HashSet::new();
    while let Some(p) = stack.pop() {
        if !seen.insert(p) {
            continue;
        }
        if let Some(proc_) = procs.get(&p) {
            let name = proc_.name().to_string_lossy().to_lowercase();
            let cmd = proc_
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_lowercase())
                .collect::<Vec<_>>()
                .join(" ");
            // node 跑 claude-code 时进程名是 node，命令行里含 claude
            if name.contains("claude") || cmd.contains("claude") {
                return true;
            }
        }
        for (cpid, cproc) in procs {
            if cproc.parent() == Some(p) {
                stack.push(*cpid);
            }
        }
    }
    false
}

// 读 statusLine 采集脚本写的缓存
fn read_statusline_cache() -> Option<serde_json::Value> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let f = PathBuf::from(home).join(".claude").join("statusline-cache.json");
    let content = std::fs::read_to_string(f).ok()?;
    serde_json::from_str(&content).ok()
}

// resets_at 归一化到 epoch ms（兼容数字秒与 ISO 字符串）
fn reset_to_ms(v: &serde_json::Value) -> i64 {
    if let Some(n) = v.as_f64() {
        return (n * 1000.0) as i64;
    }
    if let Some(s) = v.as_str() {
        if let Ok(dt) = s.parse::<DateTime<Utc>>() {
            return dt.timestamp_millis();
        }
    }
    0
}

// 前端每隔十几秒轮询一次；传当前活跃标签的 session_id 做进程检测
#[tauri::command]
fn usage_stats(manager: State<'_, PtyManager>, session_id: String) -> UsageStats {
    let mut stats = UsageStats::default();

    // 当前标签真在跑 claude 才显示；否则前端隐藏整条
    let shell_pid = manager
        .sessions
        .lock()
        .ok()
        .and_then(|s| s.get(&session_id).and_then(|x| x.pid));
    stats.running_claude = shell_pid.map_or(false, claude_running_under);
    if !stats.running_claude {
        return stats;
    }

    let cache = match read_statusline_cache() {
        Some(c) => c,
        None => return stats,
    };
    stats.has_data = true;

    if let Some(m) = cache["model"]["display_name"]
        .as_str()
        .or_else(|| cache["model"]["id"].as_str())
    {
        stats.model = m.to_string();
    }
    stats.context_pct = cache["context_window"]["used_percentage"]
        .as_f64()
        .unwrap_or(0.0);

    let rl = &cache["rate_limits"];
    if rl.is_object() {
        stats.has_rate_limits = true;
        stats.five_hour_pct = rl["five_hour"]["used_percentage"].as_f64().unwrap_or(0.0);
        stats.five_hour_reset_ms = reset_to_ms(&rl["five_hour"]["resets_at"]);
        stats.seven_day_pct = rl["seven_day"]["used_percentage"].as_f64().unwrap_or(0.0);
        stats.seven_day_reset_ms = reset_to_ms(&rl["seven_day"]["resets_at"]);
    }

    if let Some(u) = cache["updated_at"].as_f64() {
        stats.cache_age_sec = (Utc::now().timestamp() as f64 - u) as i64;
    }

    stats
}

// ---------- statusLine 配置（挂/卸采集脚本到 ~/.claude/settings.json）----------

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct StatuslineStatus {
    configured: bool,        // 已挂 HyperTerminal 的采集脚本
    occupied_by_other: bool, // statusLine 已被别的命令占用
    other_command: String,   // 占用它的命令（供前端提示）
    node_available: bool,    // node 是否在 PATH（脚本要用）
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    Some(PathBuf::from(home).join(".claude").join("settings.json"))
}

// 采集脚本落地路径：打包后在 resource_dir，dev 下 tauri 也会拷到 resource_dir
fn statusline_script_path(app: &AppHandle) -> Option<PathBuf> {
    let rd = app.path().resource_dir().ok()?;
    let cands = [
        rd.join("resources").join("statusline-writer.cjs"),
        rd.join("statusline-writer.cjs"),
        rd.join("_up_").join("resources").join("statusline-writer.cjs"),
    ];
    cands.into_iter().find(|p| p.exists())
}

#[tauri::command]
fn statusline_status(app: AppHandle) -> StatuslineStatus {
    let mut st = StatuslineStatus::default();
    st.node_available = which("node.exe").is_some() || which("node").is_some();
    if let Some(p) = settings_path() {
        if let Ok(content) = std::fs::read_to_string(&p) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(cmd) = v["statusLine"]["command"].as_str() {
                    if cmd.contains("statusline-writer") {
                        st.configured = true;
                    } else if !cmd.is_empty() {
                        st.occupied_by_other = true;
                        st.other_command = cmd.to_string();
                    }
                }
            }
        }
    }
    let _ = app; // resource 路径检查留给 configure 时
    st
}

#[tauri::command]
fn configure_statusline(app: AppHandle, enable: bool) -> Result<(), String> {
    let sp = settings_path().ok_or("找不到 settings.json 路径")?;
    let mut root: serde_json::Value = std::fs::read_to_string(&sp)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root.is_object() {
        root = serde_json::json!({});
    }

    if enable {
        let script = statusline_script_path(&app).ok_or("找不到采集脚本（打包资源缺失）")?;
        // 被别的 statusLine 占用则拒绝覆盖
        if let Some(cmd) = root["statusLine"]["command"].as_str() {
            if !cmd.is_empty() && !cmd.contains("statusline-writer") {
                return Err(format!("已存在其他 statusLine，未覆盖：{}", cmd));
            }
        }
        let script_str = script.display().to_string();
        // resource_dir() 在 Windows 会带 \\?\ 扩展长度前缀，node/claude 不认，去掉
        let clean = script_str.strip_prefix(r"\\?\").unwrap_or(&script_str);
        let command = format!("node \"{}\"", clean);
        root["statusLine"] = serde_json::json!({
            "type": "command",
            "command": command,
            "padding": 0
        });
    } else {
        let is_ours = root["statusLine"]["command"]
            .as_str()
            .map_or(false, |c| c.contains("statusline-writer"));
        if is_ours {
            if let Some(obj) = root.as_object_mut() {
                obj.remove("statusLine");
            }
        }
    }

    if let Some(dir) = sp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&sp, text).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(PtyManager::default())
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::apply_acrylic;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = apply_acrylic(&window, Some((18, 18, 18, 160)));
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            pty_create,
            pty_write,
            pty_resize,
            pty_close,
            list_dir,
            home_dir,
            detect_shells,
            usage_stats,
            statusline_status,
            configure_statusline
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
