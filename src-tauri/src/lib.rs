use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
const POWERSHELL_INJECT: &str =
    "function prompt { $p=(Get-Location).Path; $e=[char]27; \"$e]9;9;$p$e\\PS $p> \" }; clear\r";
// CMD：PROMPT 里 $E=ESC、$P=当前路径、$G=>；先 cls 再设，避免回显那行注入命令
const CMD_INJECT: &str = "cls & prompt $E]9;9;$P$E\\$P$G \r";
// Git Bash：PROMPT_COMMAND 每次提示符前 printf 出 OSC 9;9；pwd -W 取 Windows 路径喂文件树
const GIT_BASH_INJECT: &str =
    "export PROMPT_COMMAND='printf \"\\033]9;9;%s\\033\\\\\" \"$(pwd -W 2>/dev/null || pwd)\"'\r";
const ZSH_INJECT: &str =
    "autoload -Uz add-zsh-hook; function _brace_precmd(){ printf '\\033]9;9;%s\\033\\\\' \"$PWD\"; }; add-zsh-hook precmd _brace_precmd; clear\r";
const POSIX_BASH_INJECT: &str =
    "_brace_prompt_command='printf \"\\033]9;9;%s\\033\\\\\" \"$PWD\"'; PROMPT_COMMAND=\"$_brace_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}\"; clear\r";

fn default_shell_path() -> String {
    #[cfg(target_os = "windows")]
    {
        return "powershell.exe".to_string();
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|shell| Path::new(shell).is_file())
            .unwrap_or_else(|| "/bin/zsh".to_string())
    }
}

fn resolved_shell_type(requested: &str, executable: &str) -> String {
    if !requested.is_empty() && requested != "default" {
        return requested.to_string();
    }
    Path::new(executable)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .trim_end_matches(".exe")
        .to_lowercase()
}

// 新建终端会话。shell_path 为空时按平台选择默认 Shell；shell_type 决定 cwd 上报方式。
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
        default_shell_path()
    } else {
        shell_path
    };
    let shell_type = resolved_shell_type(&shell_type, &exe);
    let mut cmd = CommandBuilder::new(&exe);
    #[cfg(not(target_os = "windows"))]
    match shell_type.as_str() {
        "zsh" | "bash" => {
            cmd.arg("-l");
        }
        "fish" => {
            cmd.arg("--login");
        }
        _ => {}
    }
    let start_dir = if !cwd.trim().is_empty() {
        cwd
    } else {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_default()
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
        "gitbash" => GIT_BASH_INJECT,
        "bash" => POSIX_BASH_INJECT,
        "zsh" => ZSH_INJECT,
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

    manager.sessions.lock().map_err(|e| e.to_string())?.insert(
        id,
        PtySession {
            writer,
            master: pair.master,
            pid,
        },
    );
    Ok(())
}

#[tauri::command]
fn pty_write(manager: State<'_, PtyManager>, id: String, data: String) -> Result<(), String> {
    let mut sessions = manager.sessions.lock().map_err(|e| e.to_string())?;
    if let Some(s) = sessions.get_mut(&id) {
        s.writer
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
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
    for entry in std::fs::read_dir(&path)
        .map_err(|e| e.to_string())?
        .flatten()
    {
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

// ---------- Git 装饰 ----------

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct GitStatus {
    is_repo: bool,
    branch: String,
    changed_count: u32,
    files: HashMap<String, String>, // 绝对路径(\分隔) → M/A/?/D/R/!(ignored)
}

// 在 cwd 下跑 git，静默（不弹控制台窗口）
fn run_git(cwd: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(cwd).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        None
    }
}

// porcelain 两位状态码 XY → 单字符归类（优先级：删除 > 重命名 > 新增 > 改动）
fn classify(xy: &str) -> &'static str {
    if xy == "??" {
        return "?";
    }
    if xy == "!!" {
        return "!";
    }
    if xy.contains('D') {
        return "D";
    }
    if xy.contains('R') {
        return "R";
    }
    if xy.contains('A') {
        return "A";
    }
    "M"
}

#[tauri::command]
fn git_status(cwd: String) -> GitStatus {
    let mut st = GitStatus::default();
    if cwd.trim().is_empty() {
        return st;
    }
    // 取当前分支，顺带验证是否 git 仓库；不是就直接返回空
    match run_git(&cwd, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        Some(b) => {
            st.is_repo = true;
            st.branch = b.trim().to_string();
        }
        None => return st,
    }
    let top = run_git(&cwd, &["rev-parse", "--show-toplevel"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if let Some(out) = run_git(&cwd, &["status", "--porcelain", "--ignored"]) {
        for line in out.lines() {
            if line.len() < 4 {
                continue;
            }
            let xy = &line[0..2];
            let mut path = &line[3..];
            // 重命名 "old -> new" 取 new
            if let Some(idx) = path.find(" -> ") {
                path = &path[idx + 4..];
            }
            let path = path.trim_matches('"').trim_end_matches('/');
            // git 返回相对 toplevel、/ 分隔；转成绝对 + \ 分隔，跟 list_dir 一致
            let abs = format!("{}/{}", top.trim_end_matches('/'), path).replace('/', "\\");
            let code = classify(xy);
            if code != "!" {
                st.changed_count += 1;
            }
            st.files.insert(abs, code.to_string());
        }
    }
    st
}

// ---------- 文件预览 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePreview {
    kind: String,    // text | image | binary | toolarge
    content: String, // text: 文本内容；image: data URI；其他: 空
    size: u64,
}

#[tauri::command]
fn read_file(path: String) -> FilePreview {
    let empty = |kind: &str, size: u64| FilePreview {
        kind: kind.into(),
        content: String::new(),
        size,
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let ext = std::path::Path::new(&path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let is_img = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg"
    );
    if is_img {
        if size > 5_000_000 {
            return empty("toolarge", size);
        }
        if let Ok(bytes) = std::fs::read(&path) {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let mime: String = match ext.as_str() {
                "svg" => "image/svg+xml".into(),
                "jpg" | "jpeg" => "image/jpeg".into(),
                "ico" => "image/x-icon".into(),
                e => format!("image/{}", e),
            };
            return FilePreview {
                kind: "image".into(),
                content: format!("data:{};base64,{}", mime, b64),
                size,
            };
        }
        return empty("binary", size);
    }

    // 文本：超过 2MB 不预览；能按 UTF-8 读出来就当文本，否则二进制
    if size > 2_000_000 {
        return empty("toolarge", size);
    }
    match std::fs::read_to_string(&path) {
        Ok(text) => FilePreview {
            kind: "text".into(),
            content: text,
            size,
        },
        Err(_) => empty("binary", size),
    }
}

#[tauri::command]
fn write_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

// ---------- Shell 检测 ----------

#[derive(Serialize)]
struct ShellInfo {
    id: String,
    name: String,
    path: String,
    shell_type: String, // powershell | cmd | gitbash | zsh | bash | fish
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformInfo {
    os: String,
    arch: String,
}

#[tauri::command]
fn platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    }
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

    #[cfg(target_os = "windows")]
    {
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
            if let Some(root) = std::path::Path::new(&git).parent().and_then(|p| p.parent()) {
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
                shell_type: "gitbash".into(),
            });
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let configured = std::env::var("SHELL").ok();
        let candidates = [
            ("zsh", "Z shell", "/bin/zsh", "zsh"),
            ("bash", "Bash", "/bin/bash", "bash"),
            ("fish", "Fish", "/opt/homebrew/bin/fish", "fish"),
            ("fish-usr", "Fish", "/usr/local/bin/fish", "fish"),
        ];
        for (id, name, path, shell_type) in candidates {
            if Path::new(path).is_file()
                && !shells.iter().any(|shell: &ShellInfo| shell.path == path)
            {
                shells.push(ShellInfo {
                    id: id.into(),
                    name: name.into(),
                    path: path.into(),
                    shell_type: shell_type.into(),
                });
            }
        }
        if let Some(path) = configured {
            if Path::new(&path).is_file() && !shells.iter().any(|shell| shell.path == path) {
                let name = Path::new(&path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Shell")
                    .to_string();
                shells.insert(
                    0,
                    ShellInfo {
                        id: "default".into(),
                        name: format!("{} (Default)", name),
                        path,
                        shell_type: name.to_lowercase(),
                    },
                );
            } else if let Some(index) = shells.iter().position(|shell| shell.path == path) {
                let shell = shells.remove(index);
                shells.insert(0, shell);
            }
        }
    }

    shells
}

// ---------- Claude 用量统计 ----------
// 数据源：~/.claude/statusline-cache.json，由 Brace 的 statusLine 采集脚本写入
// （脚本接住 Claude Code 通过 statusLine stdin 喂的官方运行时数据）。
// 这里只负责：① 判断当前标签是否真在跑 claude；② 读缓存把官方 context/5h/7d 吐给前端。

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct UsageStats {
    agent: String, // 当前标签在跑什么："claude" / "codex" / ""（空=没跑）
    model: String,
    context_pct: f64,        // 上下文占用 %（claude 和 codex 都有）
    five_hour_pct: f64,      // claude：官方 5h 额度用量 %
    five_hour_reset_ms: i64, // claude：5h 重置时间（epoch ms，0=无）
    seven_day_pct: f64,      // claude：官方 7d 额度用量 %
    seven_day_reset_ms: i64, // claude：7d 重置时间（epoch ms，0=无）
    codex_total_tokens: u64, // codex：会话累计 token
    cache_age_sec: i64,      // claude：缓存数据距今秒数
    has_rate_limits: bool,   // claude：缓存里有没有 5h/7d 额度
    has_data: bool,          // 数据是否读到
}

// 判断给定 shell pid 的后代进程里在跑哪个 agent → "claude" / "codex" / ""
fn detect_agent(root: u32) -> String {
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
            // node 跑 claude-code、或 codex 二进制/子进程，靠进程名或命令行关键字识别
            if name.contains("claude") || cmd.contains("claude") {
                return "claude".into();
            }
            if name.contains("codex") || cmd.contains("codex") {
                return "codex".into();
            }
        }
        for (cpid, cproc) in procs {
            if cproc.parent() == Some(p) {
                stack.push(*cpid);
            }
        }
    }
    String::new()
}

// 读 statusLine 采集脚本写的缓存
fn read_statusline_cache() -> Option<serde_json::Value> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let f = PathBuf::from(home)
        .join(".claude")
        .join("statusline-cache.json");
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

// 递归收集目录下所有 jsonl
fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_jsonl(&p, out);
            } else if p.extension().map_or(false, |x| x == "jsonl") {
                out.push(p);
            }
        }
    }
}

// 读 codex 最新会话的用量 → (上下文占用%, 会话累计 token)
// 数据源：~/.codex/sessions（及 archived_sessions）下的 rollout-*.jsonl，
// 取最后一条 token_count 事件：last_token_usage.input_tokens / model_context_window 为上下文占用。
// codex 的 rate_limits 字段本地通常为 null，故不取 5h/周额度。
fn read_codex_usage() -> Option<(f64, u64)> {
    let base = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_default();
            PathBuf::from(home).join(".codex")
        });
    let mut files = Vec::new();
    for sub in ["sessions", "archived_sessions"] {
        collect_jsonl(&base.join(sub), &mut files);
    }
    // 文件路径含 ISO 时间戳（sessions/2026/07/22/rollout-2026-07-22T...），字典序最大 = 最新
    let file = files.into_iter().max()?;

    let content = std::fs::read_to_string(&file).ok()?;
    let mut result: Option<(f64, u64)> = None;
    for line in content.lines() {
        if !line.contains("token_count") {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let payload = &v["payload"];
        if payload["type"] != "token_count" {
            continue;
        }
        let info = &payload["info"];
        let window = info["model_context_window"].as_f64().unwrap_or(0.0);
        let last_input = info["last_token_usage"]["input_tokens"]
            .as_f64()
            .unwrap_or(0.0);
        let ctx = if window > 0.0 {
            (last_input / window * 100.0).min(100.0)
        } else {
            0.0
        };
        let total = info["total_token_usage"]["total_tokens"]
            .as_u64()
            .unwrap_or(0);
        result = Some((ctx, total)); // 保留最后一条
    }
    result
}

// 前端每隔十几秒轮询一次；传当前活跃标签的 session_id 做进程检测
#[tauri::command]
fn usage_stats(manager: State<'_, PtyManager>, session_id: String) -> UsageStats {
    let mut stats = UsageStats::default();

    // 当前标签在跑什么 agent；都没跑就返回空，前端隐藏整条
    let shell_pid = manager
        .sessions
        .lock()
        .ok()
        .and_then(|s| s.get(&session_id).and_then(|x| x.pid));
    stats.agent = shell_pid.map(detect_agent).unwrap_or_default();

    match stats.agent.as_str() {
        "claude" => {
            let cache = match read_statusline_cache() {
                Some(c) => c,
                None => return stats,
            };
            // 新鲜度：缓存超过 15 分钟没更新（claude 没在活跃跑，或开关已关脚本停写），
            // 不拿旧数据糊弄——直接返回，前端隐藏整条
            let age = cache["updated_at"]
                .as_f64()
                .map_or(f64::INFINITY, |u| Utc::now().timestamp() as f64 - u);
            if age > 900.0 {
                return stats;
            }
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
        }
        "codex" => {
            if let Some((ctx, total)) = read_codex_usage() {
                stats.has_data = true;
                stats.model = "Codex".into();
                stats.context_pct = ctx;
                stats.codex_total_tokens = total;
            }
        }
        _ => {}
    }

    stats
}

// ---------- statusLine 配置（挂/卸采集脚本到 ~/.claude/settings.json）----------

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct StatuslineStatus {
    configured: bool,        // 已挂 Brace 的采集脚本
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
        rd.join("_up_")
            .join("resources")
            .join("statusline-writer.cjs"),
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
fn configure_statusline(app: AppHandle, enable: bool, force: bool) -> Result<(), String> {
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
        // 被别的 statusLine 占用：force=false 拒绝并提示，force=true 强制接管覆盖
        if !force {
            if let Some(cmd) = root["statusLine"]["command"].as_str() {
                if !cmd.is_empty() && !cmd.contains("statusline-writer") {
                    return Err(format!("已存在其他 statusLine，未覆盖：{}", cmd));
                }
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
        // 卸载时删掉残留缓存，避免关开关后前端仍读到旧数据继续显示
        if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            let _ = std::fs::remove_file(
                PathBuf::from(home)
                    .join(".claude")
                    .join("statusline-cache.json"),
            );
        }
    }

    if let Some(dir) = sp.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&sp, text).map_err(|e| e.to_string())?;
    Ok(())
}

// Win11 判断（build >= 22000）。Win10 的 acrylic 亚克力有边缘黑边 + 拖动卡顿，需区分。
#[cfg(target_os = "windows")]
fn is_win11() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .ok()
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber").ok())
        .and_then(|b| b.parse::<u32>().ok())
        .map_or(false, |n| n >= 22000)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(PtyManager::default())
        .setup(|_app| {
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::apply_acrylic;
                if let Some(window) = _app.get_webview_window("main") {
                    // 只有 Win11 才上 acrylic；Win10 的 acrylic 边缘有黑边、拖动卡，
                    // 退回普通背景层（窗口正常，只是少了那层毛玻璃）
                    if is_win11() {
                        let _ = apply_acrylic(&window, Some((18, 18, 18, 160)));
                    }
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
            configure_statusline,
            git_status,
            read_file,
            write_file,
            platform_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::resolved_shell_type;

    #[test]
    fn resolves_default_unix_shell_from_executable() {
        assert_eq!(resolved_shell_type("default", "/bin/zsh"), "zsh");
        assert_eq!(resolved_shell_type("", "/bin/bash"), "bash");
    }

    #[test]
    fn preserves_explicit_shell_type() {
        assert_eq!(
            resolved_shell_type("powershell", "C:\\Windows\\pwsh.exe"),
            "powershell"
        );
    }
}
