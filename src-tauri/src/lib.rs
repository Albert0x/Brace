use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

// 单个终端会话：持有写入端、主控端与可 kill 的子进程句柄
struct PtySession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    pid: Option<u32>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
}

// 流式 UTF-8 解码：把跨 read() 块被截断的多字节序列留到下一块，避免中文/emoji 花屏。
// leftover 里最多滞留 3 个字节（UTF-8 最长 4 字节，末尾不完整序列 <=3 字节待续）。
// 遇到真正非法字节（并非只是截断）时用替换字符跳过，不会无限攒积。
fn decode_utf8_stream(leftover: &mut Vec<u8>, new_bytes: &[u8]) -> String {
    leftover.extend_from_slice(new_bytes);
    let mut out = String::new();
    loop {
        match std::str::from_utf8(leftover) {
            Ok(s) => {
                out.push_str(s);
                leftover.clear();
                break;
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to > 0 {
                    out.push_str(std::str::from_utf8(&leftover[..valid_up_to]).unwrap());
                }
                match e.error_len() {
                    Some(bad_len) => {
                        out.push('\u{FFFD}');
                        leftover.drain(..valid_up_to + bad_len);
                        // 继续处理剩余字节，可能还有合法内容或新的截断尾巴
                    }
                    None => {
                        leftover.drain(..valid_up_to);
                        break;
                    }
                }
            }
        }
    }
    out
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
const BASH_INJECT: &str =
    "export PROMPT_COMMAND='printf \"\\033]9;9;%s\\033\\\\\" \"$(pwd -W 2>/dev/null || pwd)\"'\r";

// 新建一个终端会话。shell_path 为空回退 powershell.exe；shell_type 决定是否注入 cwd 上报。
// 参数偏多，但这是 IPC 契约：tauri command 的入参按名字从前端对象里取，
// 打包成结构体只会让前端调用多一层嵌套，换不来实际可读性
#[allow(clippy::too_many_arguments)]
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
    // 注入当前选中的配置组（AI 中转端点、代理等）。env() 是在继承来的环境之上覆盖，
    // 所以没配的变量保持系统原值。只对新建的会话生效——已经跑起来的进程改不了环境变量，
    // 这是操作系统的规矩，不是这里偷懒
    for (k, v) in active_env(&app) {
        cmd.env(k, v);
    }
    let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let pid = child.process_id();
    let child: Arc<Mutex<Box<dyn Child + Send + Sync>>> = Arc::new(Mutex::new(child));

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
    let child_for_wait = Arc::clone(&child);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut leftover: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = decode_utf8_stream(&mut leftover, &buf[..n]);
                    if !data.is_empty() {
                        let _ = app_handle.emit(
                            "pty-output",
                            PtyOutput {
                                id: sid.clone(),
                                data,
                            },
                        );
                    }
                }
                Err(_) => break,
            }
        }
        // 读循环结束说明进程已退出（或即将退出），在这里回收，避免 kill() 和 wait() 抢锁死锁
        if let Ok(mut c) = child_for_wait.lock() {
            let _ = c.wait();
        }
        let _ = app_handle.emit("pty-exit", sid.clone());
    });

    manager.sessions.lock().map_err(|e| e.to_string())?.insert(
        id,
        PtySession {
            writer,
            master: pair.master,
            pid,
            child,
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

// 干掉一个会话的整棵进程树。shell 本身先 kill 一次兜底；Windows 下 shell 里跑起来的
// 子进程（node/claude 等）不会被这个 kill 连坐，必须再用 taskkill /T 杀掉整棵树
fn kill_session(session: &PtySession) {
    if let Ok(mut c) = session.child.lock() {
        let _ = c.kill();
    }
    #[cfg(windows)]
    if let Some(pid) = session.pid {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output();
    }
}

#[tauri::command]
fn pty_close(manager: State<'_, PtyManager>, id: String) -> Result<(), String> {
    let session = manager
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id);
    if let Some(session) = session {
        kill_session(&session);
    }
    Ok(())
}

// 应用退出时的兜底清理：点窗口 × 关闭时 React 组件不走卸载流程，pty_close 一次都不会被调，
// shell 里跑着的 node/claude 会留在后台当孤儿进程。这里在事件循环退出前全量收尸。
// 先把 map 整个取出来再逐个 kill，避免 taskkill 期间一直占着锁
fn kill_all_sessions(manager: &PtyManager) {
    let sessions = match manager.sessions.lock() {
        Ok(mut s) => std::mem::take(&mut *s),
        Err(e) => std::mem::take(&mut *e.into_inner()), // 有线程 panic 过也照样收尸
    };
    for (_, session) in sessions {
        kill_session(&session);
    }
}

// ---------- 文件系统 ----------

#[derive(Serialize)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    hidden: bool,
}

// 是否隐藏。Windows 上「隐藏」是文件属性，跟文件名以点开头没有半点关系——
// .gitignore / .env / .github 在资源管理器里都是正常显示的，按点前缀过滤会把
// 开发者最常看的那批文件全藏起来。反过来 .git 目录 git 自己设了隐藏属性，
// 按属性判断刚好把它挡在外面，跟资源管理器表现一致
fn is_hidden(entry: &std::fs::DirEntry) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        // 读不到属性（权限不足等）就当它不隐藏，宁可多显示也别凭空藏东西
        entry
            .metadata()
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        entry.file_name().to_string_lossy().starts_with('.')
    }
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
            hidden: is_hidden(&entry),
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

// 在 cwd 下跑 git，静默（不弹控制台窗口）；core.quotepath=false 让中文/特殊字符路径
// 原样输出，不被 octal 转义成 "\346\226\207..." 这种跟 list_dir 的路径对不上的形式
// 构造一条 git 命令。
//
// read_only 的调用会带上 --no-optional-locks：git status 默认会顺手刷新索引并写回，
// 而那需要 index.lock。Brace 每 20 秒轮询一次状态，用户正好在终端里敲 git commit
// 就会撞上「Unable to create index.lock」——终端自己把用户的 git 命令搞挂了，
// 而且没人会想到是终端干的。
//
// 代价是这个开关也禁止了索引缓存的刷新，个别文件 stat 过期时会被多报一次"已修改"。
// 接受这个代价：装饰上多一个标记是显示问题，用户的 git 命令随机失败是功能故障。
// 写操作（add/commit/push）不带这个开关，它们本就该拿锁。
fn git_cmd(cwd: &str, args: &[&str], read_only: bool) -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    if read_only {
        cmd.arg("--no-optional-locks");
    }
    cmd.arg("-c").arg("core.quotepath=false");
    cmd.arg("-C").arg(cwd).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn run_git(cwd: &str, args: &[&str]) -> Option<String> {
    let out = git_cmd(cwd, args, true).output().ok()?;
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
    // -z：记录以 NUL 分隔，路径不加引号/不转义；重命名/拷贝记录是两个 NUL 分隔字段
    // "XY newpath\0oldpath\0"，要多吃一个 token 跳过旧路径
    if let Some(out) = run_git(&cwd, &["status", "--porcelain", "-z", "--ignored"]) {
        let mut tokens = out.split('\0');
        while let Some(rec) = tokens.next() {
            if rec.len() < 4 {
                continue;
            }
            let xy = &rec[0..2];
            let path = rec[3..].trim_end_matches('/');
            if xy.contains('R') || xy.contains('C') {
                tokens.next(); // 跳过旧路径
            }
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

// 跑 git 拿结果：Ok=stdout，Err=git 的 stderr（失败原因原样给前端，如未配 user.name、
// push 被拒、无 upstream 等）。run_git 只返回 Option 丢了错误，提交场景必须拿到原因
fn run_git_out(cwd: &str, args: &[&str], read_only: bool) -> Result<String, String> {
    let out = git_cmd(cwd, args, read_only)
        .output()
        .map_err(|e| format!("无法运行 git：{}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        let so = String::from_utf8_lossy(&out.stdout).to_string();
        Err(if err.trim().is_empty() { so } else { err })
    }
}

// git 在 Windows 上认正斜杠，而 git_status 给前端的是 \ 分隔的绝对路径。
// 传回来当 pathspec 用之前统一转一下
fn to_pathspec(path: &str) -> String {
    path.replace('\\', "/")
}

// 提交。all=true 走 add -A（全选是最常见的场景，也避开了一长串路径把命令行撑爆的问题）；
// 否则只 add/commit 选中的那些路径。
//
// 部分提交时 commit 也带 pathspec，这一点很关键：用户可能已经在终端里 git add 过别的东西，
// 不带 pathspec 的 commit 会把那些一并提交掉——而界面上根本没勾它们。
#[tauri::command]
fn git_commit(
    cwd: String,
    message: String,
    push: bool,
    paths: Vec<String>,
    all: bool,
) -> Result<String, String> {
    if cwd.trim().is_empty() {
        return Err("没有工作目录".into());
    }
    if message.trim().is_empty() {
        return Err("提交信息不能为空".into());
    }

    if all {
        run_git_out(&cwd, &["add", "-A"], false)?;
        run_git_out(&cwd, &["commit", "-m", &message], false)?;
    } else {
        if paths.is_empty() {
            return Err("没有选中任何文件".into());
        }
        let specs: Vec<String> = paths.iter().map(|p| to_pathspec(p)).collect();
        // add 要能处理已删除的文件，-A 配 pathspec 正是「把这些路径的增删改都暂存」
        let mut add: Vec<&str> = vec!["add", "-A", "--"];
        add.extend(specs.iter().map(|s| s.as_str()));
        run_git_out(&cwd, &add, false)?;

        let mut commit: Vec<&str> = vec!["commit", "-m", &message, "--"];
        commit.extend(specs.iter().map(|s| s.as_str()));
        run_git_out(&cwd, &commit, false)?;
    }

    if push {
        run_git_out(&cwd, &["push"], false)?;
        Ok("pushed".into())
    } else {
        Ok("committed".into())
    }
}

// 单文件相对 HEAD 的 diff。提交前至少要能看见自己在提交什么
#[tauri::command]
fn git_diff(cwd: String, path: String) -> Result<String, String> {
    if cwd.trim().is_empty() || path.trim().is_empty() {
        return Err("参数为空".into());
    }
    let spec = to_pathspec(&path);

    // 未跟踪的文件 git diff 给不出东西（HEAD 里没有它）。这类文件整份都是新增，
    // 直接读出来自己拼成 diff 的样子，比让用户看一片空白强
    let tracked = run_git_out(&cwd, &["ls-files", "--error-unmatch", "--", &spec], true).is_ok();
    if !tracked {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() > 512_000 {
            return Ok("(新文件过大，不显示内容)".into());
        }
        let Some((text, _)) = decode_text(&bytes) else {
            return Ok("(二进制文件)".into());
        };
        let body: String = text
            .lines()
            .map(|l| format!("+{}\n", l))
            .collect::<Vec<_>>()
            .join("");
        return Ok(format!("@@ 新文件 @@\n{}", body));
    }

    // HEAD 在一个提交都还没有的仓库里不存在，这时跟空树比
    let diff = match run_git_out(&cwd, &["diff", "HEAD", "--", &spec], true) {
        Ok(d) => d,
        Err(_) => run_git_out(&cwd, &["diff", "--", &spec], true)?,
    };
    if diff.len() > 512_000 {
        return Ok("(改动过大，不显示 diff)".into());
    }
    Ok(diff)
}

// ---------- 文件预览 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePreview {
    kind: String,    // text | image | binary | toolarge
    content: String, // text: 文本内容；image: data URI；其他: 空
    size: u64,
    encoding: String, // text: 原始编码，保存时按它写回；其他: 空
}

// 编码标签。UTF-16 由我们自己处理（encoding_rs 不支持 encode 到 UTF-16），
// 其余走 encoding_rs 的规范名（"UTF-8" / "GBK" / "Shift_JIS" / "windows-1252"…）
const ENC_UTF8: &str = "UTF-8";
const ENC_UTF8_BOM: &str = "UTF-8-BOM";
const ENC_UTF16LE: &str = "UTF-16LE";
const ENC_UTF16BE: &str = "UTF-16BE";

// 按 BOM 判编码；返回 (编码标签, BOM 长度)
fn sniff_bom(b: &[u8]) -> Option<(&'static str, usize)> {
    if b.starts_with(&[0xEF, 0xBB, 0xBF]) {
        Some((ENC_UTF8_BOM, 3))
    } else if b.starts_with(&[0xFF, 0xFE]) {
        Some((ENC_UTF16LE, 2))
    } else if b.starts_with(&[0xFE, 0xFF]) {
        Some((ENC_UTF16BE, 2))
    } else {
        None
    }
}

// UTF-16 解码（BOM 已剥离）。奇数个字节说明文件截断，末尾半个码元直接丢掉
fn decode_utf16(body: &[u8], little: bool) -> String {
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if little {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

// 字节 → (文本, 编码标签)。判不出文本则返回 None（交给调用方当二进制处理）。
// 顺序：BOM → NUL 探测 → 严格 UTF-8 → chardetng 嗅探
fn decode_text(bytes: &[u8]) -> Option<(String, String)> {
    if let Some((enc, skip)) = sniff_bom(bytes) {
        let body = &bytes[skip..];
        return Some(match enc {
            ENC_UTF16LE => (decode_utf16(body, true), enc.into()),
            ENC_UTF16BE => (decode_utf16(body, false), enc.into()),
            _ => (String::from_utf8_lossy(body).into_owned(), enc.into()),
        });
    }
    // NUL 字节基本可以断定是二进制。必须放在 BOM 判断之后——UTF-16 里的 ASCII
    // 字符高位字节全是 0x00，先查 NUL 会把 UTF-16 文本全部误杀
    if bytes.iter().take(8192).any(|&b| b == 0) {
        return None;
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Some((s.to_string(), ENC_UTF8.into()));
    }
    // 不是合法 UTF-8：嗅探（对 GBK/Big5/Shift_JIS 这些中日韩编码识别率还行）。
    // ISO-2022-JP 用转义序列表示，字节全在 ASCII 范围内，合法 UTF-8 那步就已经拦下了，
    // 走到这儿再允许它只会增加误判，故 Deny
    use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
    let mut det = EncodingDetector::new(Iso2022JpDetection::Deny);
    det.feed(bytes, true);
    let enc = det.guess(None, Utf8Detection::Allow);
    let (text, _, had_errors) = enc.decode(bytes);
    // 猜的编码解出来还是一堆替换字符，说明根本不是文本，别硬凑
    if had_errors && text.matches('\u{FFFD}').count() * 20 > text.chars().count() {
        return None;
    }
    Some((text.into_owned(), enc.name().to_string()))
}

#[tauri::command]
fn read_file(path: String) -> FilePreview {
    let empty = |kind: &str, size: u64| FilePreview {
        kind: kind.into(),
        content: String::new(),
        size,
        encoding: String::new(),
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
                encoding: String::new(),
            };
        }
        return empty("binary", size);
    }

    // 文本：超过 2MB 不预览。编码不限 UTF-8——GBK / UTF-16 也照样认，
    // 并把识别出的编码带回前端，保存时原样写回，不静默转码
    if size > 2_000_000 {
        return empty("toolarge", size);
    }
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return empty("binary", size),
    };
    match decode_text(&bytes) {
        Some((text, encoding)) => FilePreview {
            kind: "text".into(),
            content: text,
            size,
            encoding,
        },
        None => empty("binary", size),
    }
}

// 按指定编码把文本编码成字节。encoding 为空或未知时退回 UTF-8。
// 返回 None 表示该编码无法表达这段文本（例如往 GBK 里塞 emoji），由调用方报错，
// 绝不静默用 '?' 替换掉用户的字符
fn encode_text(content: &str, encoding: &str) -> Option<Vec<u8>> {
    match encoding {
        ENC_UTF8_BOM => {
            let mut v = vec![0xEF, 0xBB, 0xBF];
            v.extend_from_slice(content.as_bytes());
            Some(v)
        }
        ENC_UTF16LE | ENC_UTF16BE => {
            let little = encoding == ENC_UTF16LE;
            let mut v = if little {
                vec![0xFF, 0xFE]
            } else {
                vec![0xFE, 0xFF]
            };
            for u in content.encode_utf16() {
                v.extend_from_slice(&if little {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                });
            }
            Some(v)
        }
        "" | ENC_UTF8 => Some(content.as_bytes().to_vec()),
        name => match encoding_rs::Encoding::for_label(name.as_bytes()) {
            Some(enc) => {
                let (bytes, _, had_errors) = enc.encode(content);
                if had_errors {
                    None
                } else {
                    Some(bytes.into_owned())
                }
            }
            None => Some(content.as_bytes().to_vec()),
        },
    }
}

#[tauri::command]
fn write_file(path: String, content: String, encoding: Option<String>) -> Result<(), String> {
    let enc = encoding.unwrap_or_default();
    let bytes = encode_text(&content, &enc)
        .ok_or_else(|| format!("有字符无法用原编码 {} 保存，请先把文件转成 UTF-8", enc))?;
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
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
            shell_type: "bash".into(),
        });
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
    use std::collections::{HashMap, HashSet};
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let procs = sys.processes();

    // 一次扫描建 parent → children 映射；原来每弹出一个节点就把全表再扫一遍找子进程，
    // 深度 D 时是 O(N*D)，进程一多就退化成 O(N²)
    let mut children_of: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, proc_) in procs {
        if let Some(parent) = proc_.parent() {
            children_of.entry(parent).or_default().push(*pid);
        }
    }

    let mut stack = vec![Pid::from_u32(root)];
    let mut seen = HashSet::new();
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
        if let Some(children) = children_of.get(&p) {
            stack.extend(children);
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
            } else if p.extension().is_some_and(|x| x == "jsonl") {
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
    let mut st = StatuslineStatus {
        node_available: which("node.exe").is_some() || which("node").is_some(),
        ..Default::default()
    };
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
    // 文件不存在视为全新配置；文件存在但解析失败，绝不能当 {} 处理再覆盖写回——
    // 那样会把用户已有的其他配置项全部抹掉，必须直接报错中止
    let mut root: serde_json::Value = match std::fs::read_to_string(&sp) {
        Ok(content) => serde_json::from_str(&content)
            .map_err(|e| format!("settings.json 解析失败，为避免覆盖已有配置已中止：{}", e))?,
        Err(_) => serde_json::json!({}),
    };
    if !root.is_object() {
        return Err("settings.json 顶层不是对象，为避免破坏已有配置已中止".into());
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
            .is_some_and(|c| c.contains("statusline-writer"));
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
    // 写入前备份原文件；正文走临时文件 + rename，避免写到一半崩溃/断电导致 settings.json 损坏
    if sp.exists() {
        let _ = std::fs::copy(&sp, sp.with_extension("json.bak"));
    }
    let text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    let tmp = sp.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &sp).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------- 文件树自动刷新 ----------
// 监听当前目录，有变化就 emit 一个信号，前端据此重新拉目录内容。
// 在此之前，终端里 mkdir 完左边的树是纹丝不动的，只能手动点 ⟳。
//
// **刻意只监听一层，不递归。** 递归看着更周到，但 cwd 常常就是用户主目录，
// 递归监听 C:\Users\xxx 会把 AppData、OneDrive、浏览器缓存的写入全收进来，
// 事件量大到没有意义。真要覆盖子目录，正确做法是前端把「当前展开了哪些目录」
// 报上来、逐个非递归监听——那需要先把 TreeNode 里各自为政的 expanded 状态收拢到
// 上层，属于另一件事。现在的取舍是：根目录的增删改自动刷新，子目录留给 ⟳ 按钮。

type DirWatcher = notify_debouncer_full::Debouncer<
    notify_debouncer_full::notify::RecommendedWatcher,
    notify_debouncer_full::RecommendedCache,
>;

#[derive(Default)]
struct WatchState {
    debouncer: Option<DirWatcher>,
    watched: std::collections::HashSet<String>,
}

// 监听器和已监听集合放同一把锁下，省掉两把锁的加锁顺序问题
#[derive(Default)]
struct FsWatcher(Mutex<WatchState>);

// 监听目录数上限。展开 64 个目录已经远超正常使用，真到了这个量级也说明
// 再多盯几个也没意义，不如给个明确的天花板
const MAX_WATCHED_DIRS: usize = 64;

#[tauri::command]
fn watch_dirs(
    app: AppHandle,
    state: State<'_, FsWatcher>,
    paths: Vec<String>,
) -> Result<(), String> {
    use notify_debouncer_full::notify::RecursiveMode;
    use notify_debouncer_full::{new_debouncer, DebounceEventResult};

    // 前端按「根目录在前」的顺序给，截断时保住最重要的那些
    let wanted: std::collections::HashSet<String> = paths
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .take(MAX_WATCHED_DIRS)
        .collect();

    let mut st = self_lock(&state.0)?;
    if wanted.is_empty() {
        st.debouncer = None; // drop 即停线程
        st.watched.clear();
        return Ok(());
    }

    if st.debouncer.is_none() {
        let app_handle = app.clone();
        // 500ms 防抖：一次 git checkout / pnpm install 能刷出成千上万个事件，
        // 逐个发到前端等于自己 DoS 自己。前端只关心"变了"，不关心变了什么
        st.debouncer = Some(
            new_debouncer(
                std::time::Duration::from_millis(500),
                None,
                move |res: DebounceEventResult| {
                    if res.is_ok_and(|events| !events.is_empty()) {
                        let _ = app_handle.emit("fs-change", ());
                    }
                },
            )
            .map_err(|e| e.to_string())?,
        );
        st.watched.clear(); // 新建的监听器什么都还没盯
    }

    // 增量更新：只动差集，别每次都把所有目录重新注册一遍
    let to_remove: Vec<String> = st
        .watched
        .iter()
        .filter(|p| !wanted.contains(*p))
        .cloned()
        .collect();
    let to_add: Vec<String> = wanted
        .iter()
        .filter(|p| !st.watched.contains(*p))
        .cloned()
        .collect();
    let Some(deb) = st.debouncer.as_mut() else {
        return Ok(());
    };
    for p in to_remove {
        let _ = deb.unwatch(Path::new(&p));
    }
    for p in to_add {
        // 目录可能刚被删掉/改名，注册失败跳过就行，不该让整批监听失败
        let _ = deb.watch(Path::new(&p), RecursiveMode::NonRecursive);
    }
    st.watched = wanted;
    Ok(())
}

// 锁中毒（某个线程 panic 过）时照常拿到数据继续用：这里的状态只是"在盯哪些目录"，
// 没有会被破坏的不变量，为它整个功能失效不划算
fn self_lock(m: &Mutex<WatchState>) -> Result<std::sync::MutexGuard<'_, WatchState>, String> {
    Ok(m.lock().unwrap_or_else(|e| e.into_inner()))
}

// ---------- 文件操作 ----------

// Windows 文件名限制。交给 fs 报错的话用户看到的是"系统找不到指定的路径"这种
// 毫无意义的提示，不如自己先拦下来说清楚
fn invalid_file_name(name: &str) -> Option<String> {
    let n = name.trim();
    if n.is_empty() {
        return Some("名称不能为空".into());
    }
    if n.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']) {
        return Some(r#"名称不能包含 < > : " / \ | ? *"#.into());
    }
    if n.ends_with('.') || n.ends_with(' ') {
        return Some("名称不能以点或空格结尾".into());
    }
    // CON.txt 一样是保留名，要看第一段而不是整个名字
    let stem = n.split('.').next().unwrap_or(n).to_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        return Some(format!("{} 是 Windows 保留名", stem));
    }
    None
}

#[tauri::command]
fn create_entry(parent: String, name: String, is_dir: bool) -> Result<String, String> {
    if let Some(e) = invalid_file_name(&name) {
        return Err(e);
    }
    let path = Path::new(&parent).join(name.trim());
    if path.exists() {
        return Err("同名文件或文件夹已存在".into());
    }
    if is_dir {
        std::fs::create_dir(&path).map_err(|e| e.to_string())?;
    } else {
        // create_new 而不是 File::create：后者会把已存在的文件截断成空的。
        // 上面虽然查过 exists，但那之后到这里之间仍有窗口
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
    }
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn rename_entry(path: String, name: String) -> Result<String, String> {
    if let Some(e) = invalid_file_name(&name) {
        return Err(e);
    }
    let src = Path::new(&path);
    let parent = src.parent().ok_or("这个路径没有父目录")?;
    let dst = parent.join(name.trim());
    if dst == src {
        return Ok(path);
    }
    // fs::rename 在 Windows 上会直接覆盖同名文件，先自己挡一道。
    // 但只改大小写（readme.md → README.md）时 exists() 也是 true——
    // Windows 文件系统不区分大小写，那种改名是合法的，不能挡
    let only_case_differs = dst
        .to_string_lossy()
        .eq_ignore_ascii_case(&src.to_string_lossy());
    if !only_case_differs && dst.exists() {
        return Err("同名文件或文件夹已存在".into());
    }
    std::fs::rename(src, &dst).map_err(|e| e.to_string())?;
    Ok(dst.to_string_lossy().to_string())
}

// 删除走回收站。不提供永久删除——真要彻底删，终端就在旁边
#[tauri::command]
fn delete_entry(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    trash::delete(&path).map_err(|e| e.to_string())
}

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
struct DebugLogInfo {
    path: String,
    size: u64,
    exists: bool,
}

#[tauri::command]
fn debug_log_info(app: AppHandle) -> Result<DebugLogInfo, String> {
    let path = debug_log_path(&app)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(DebugLogInfo {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
        size,
    })
}

#[tauri::command]
fn append_debug_log(app: AppHandle, lines: Vec<String>) -> Result<(), String> {
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
fn clear_debug_log(app: AppHandle) -> Result<(), String> {
    let path = debug_log_path(&app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- 背景图 ----------
// 原来整张图的 base64 直接塞 localStorage，5MB 配额一超就静默失败，
// 用户设完壁纸重启发现没了还不知道为什么。改成落盘到应用配置目录。
//
// 存的是 data URI 文本而不是原始字节：省掉 mime 猜测和扩展名映射那一整套，
// 代价只是磁盘上多占 33%——一张壁纸而已，不值得为这点体积增加复杂度。

fn bg_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到配置目录：{}", e))?;
    Ok(dir.join("background.dataurl"))
}

// data_url 传空串 = 清除背景
#[tauri::command]
fn save_bg_image(app: AppHandle, data_url: String) -> Result<(), String> {
    let path = bg_path(&app)?;
    if data_url.is_empty() {
        // 本来就没有也算成功，不用让前端去区分"没设过"和"删失败"
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data_url).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn load_bg_image(app: AppHandle) -> Option<String> {
    std::fs::read_to_string(bg_path(&app).ok()?).ok()
}

// ---------- 环境变量配置组（Profiles）----------
// 一组「名字 → 环境变量」的配置，新建终端时注入当前选中的那组。
// 用来做 AI 中转 API 切换（ANTHROPIC_BASE_URL / AUTH_TOKEN）、代理切换（HTTP_PROXY）等。
// 刻意不做成 claude/codex 两套硬编码表单——统一成环境变量组，配 gemini-cli、aider
// 甚至任何认环境变量的 CLI 都是同一套代码，预设模板只是往表里填几个 key 而已。
//
// 安全边界：secret 字段的明文只在 Rust 侧存在（落盘用 DPAPI 加密，注入时才解密），
// 永远不回传给 webview。前端只知道"有没有值"，改密钥就整个覆盖。

// DPAPI 附加熵：同一台机器同一个用户下的别的程序，光有密文也解不开
#[cfg(windows)]
const DPAPI_ENTROPY: &[u8] = b"brace.profiles.v1";
// 加密值在 JSON 里的前缀。没有这个前缀就按明文处理——用户手改配置文件直接写明文
// 也能用，下次保存时会自动加密回去
const ENC_PREFIX: &str = "enc:";

// DPAPI 加解密。protect=true 加密，false 解密。失败返回 None
#[cfg(windows)]
fn dpapi(input: &[u8], protect: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
    };
    let in_blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY.len() as u32,
        pbData: DPAPI_ENTROPY.as_ptr() as *mut u8,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: 两个入参 blob 指向的缓冲区在调用期间都存活；API 只读它们。
    // 输出 blob 由 crypt32 用 LocalAlloc 分配，拷贝完立刻 LocalFree
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &in_blob,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &in_blob,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                0,
                &mut out,
            )
        }
    };
    if ok == 0 || out.pbData.is_null() {
        return None;
    }
    let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe {
        LocalFree(out.pbData as _);
    }
    Some(data)
}

// 非 Windows 暂时没有等价的用户级密钥存储（macOS 该走 Keychain），先不假装加密
#[cfg(not(windows))]
fn dpapi(_input: &[u8], _protect: bool) -> Option<Vec<u8>> {
    None
}

// 明文 → 落盘形态。加密不可用时退回明文，不阻塞用户使用
fn seal(plain: &str) -> String {
    use base64::Engine;
    match dpapi(plain.as_bytes(), true) {
        Some(blob) => format!(
            "{}{}",
            ENC_PREFIX,
            base64::engine::general_purpose::STANDARD.encode(blob)
        ),
        None => plain.to_string(),
    }
}

// 落盘形态 → 明文。解密失败（换了机器或换了 Windows 用户）返回 None，
// 调用方按"这个密钥没了，需要重填"处理，不把密文当明文注进环境变量
fn unseal(stored: &str) -> Option<String> {
    use base64::Engine;
    let Some(b64) = stored.strip_prefix(ENC_PREFIX) else {
        return Some(stored.to_string()); // 用户手写的明文
    };
    let blob = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let plain = dpapi(&blob, false)?;
    String::from_utf8(plain).ok()
}

// ----- 落盘结构 -----

#[derive(Serialize, Deserialize, Clone, Default)]
struct StoredVar {
    key: String,
    value: String, // secret 时为 "enc:<base64>"
    #[serde(default)]
    secret: bool,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct StoredProfile {
    id: String,
    name: String,
    #[serde(default)]
    vars: Vec<StoredVar>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredStore {
    #[serde(default)]
    profiles: Vec<StoredProfile>,
    #[serde(default)]
    active_id: String, // 空 = 不注入任何东西
}

// ----- 前端交互结构（secret 明文不出后端）-----

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiVar {
    key: String,
    value: String, // secret 时恒为空
    secret: bool,
    has_value: bool, // 后端存着值没有（供 UI 显示"已保存"还是"未设置"）
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiProfile {
    id: String,
    name: String,
    vars: Vec<UiVar>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UiStore {
    profiles: Vec<UiProfile>,
    active_id: String,
    encryption_available: bool, // false 时 UI 要提示密钥是明文存的
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InVar {
    key: String,
    value: String, // secret 且为空 = 沿用已存的值，不是"清空"
    secret: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InProfile {
    id: String,
    name: String,
    vars: Vec<InVar>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InStore {
    profiles: Vec<InProfile>,
    active_id: String,
}

fn profiles_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到配置目录：{}", e))?;
    Ok(dir.join("profiles.json"))
}

fn read_store(app: &AppHandle) -> StoredStore {
    profiles_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
fn load_profiles(app: AppHandle) -> UiStore {
    let store = read_store(&app);
    UiStore {
        profiles: store
            .profiles
            .into_iter()
            .map(|p| UiProfile {
                id: p.id,
                name: p.name,
                vars: p
                    .vars
                    .into_iter()
                    .map(|v| UiVar {
                        key: v.key,
                        // 密钥明文不进 webview，只告诉前端有没有存过
                        has_value: !v.value.is_empty(),
                        value: if v.secret { String::new() } else { v.value },
                        secret: v.secret,
                    })
                    .collect(),
            })
            .collect(),
        active_id: store.active_id,
        encryption_available: cfg!(windows),
    }
}

#[tauri::command]
fn save_profiles(app: AppHandle, store: InStore) -> Result<(), String> {
    let old = read_store(&app);
    // (profileId, key) → 已存的密文，用于"前端传了空值 = 没改这个密钥"的场景
    let mut kept: HashMap<(String, String), String> = HashMap::new();
    for p in &old.profiles {
        for v in &p.vars {
            if v.secret && !v.value.is_empty() {
                kept.insert((p.id.clone(), v.key.clone()), v.value.clone());
            }
        }
    }

    let profiles: Vec<StoredProfile> = store
        .profiles
        .into_iter()
        .map(|p| {
            let vars = p
                .vars
                .into_iter()
                .map(|v| {
                    let value = if !v.secret {
                        v.value
                    } else if v.value.is_empty() {
                        // 空 = 前端没动这个密钥，沿用旧密文（前端本来也拿不到明文）
                        kept.get(&(p.id.clone(), v.key.clone()))
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        seal(&v.value)
                    };
                    StoredVar {
                        key: v.key,
                        value,
                        secret: v.secret,
                    }
                })
                .collect();
            StoredProfile {
                id: p.id,
                name: p.name,
                vars,
            }
        })
        .collect();

    let out = StoredStore {
        profiles,
        active_id: store.active_id,
    };
    let path = profiles_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 临时文件 + rename，写一半崩了也不会留下半个损坏的配置
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

// 当前选中配置组要注入的环境变量。解密失败的密钥直接跳过——
// 宁可让 claude 报"没有 token"，也不能把一串密文当 token 发出去
fn active_env(app: &AppHandle) -> Vec<(String, String)> {
    let store = read_store(app);
    if store.active_id.is_empty() {
        return Vec::new();
    }
    let Some(p) = store.profiles.iter().find(|p| p.id == store.active_id) else {
        return Vec::new();
    };
    p.vars
        .iter()
        .filter(|v| !v.key.trim().is_empty() && !v.value.is_empty())
        .filter_map(|v| unseal(&v.value).map(|plain| (v.key.trim().to_string(), plain)))
        .collect()
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
        .is_some_and(|n| n >= 22000)
}

// 状态栏显示用的真实系统信息，别再把 Win11 写死糊弄 Win10 用户
#[tauri::command]
fn os_version() -> String {
    #[cfg(target_os = "windows")]
    {
        if is_win11() {
            "Win 11".into()
        } else {
            "Win 10".into()
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::consts::OS.into()
    }
}

// 读 Windows 系统代理（HKCU\...\Internet Settings 的 ProxyEnable/ProxyServer）。
// tauri updater 的 reqwest 只认 HTTP_PROXY 环境变量、不认系统代理——国内用户开 clash
// 系统代理却收不到更新。把系统代理读出来喂给前端 check({ proxy })，一劳永逸。
#[tauri::command]
fn system_proxy() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
            .ok()?;
        let enable: u32 = key.get_value("ProxyEnable").ok()?;
        if enable == 0 {
            return None;
        }
        let server: String = key.get_value("ProxyServer").ok()?;
        // ProxyServer 两种形态："host:port" 或 "http=host:port;https=host:port;..."
        // 后者优先取 https=，其次 http=
        let addr = if server.contains('=') {
            let pick = |proto: &str| {
                server
                    .split(';')
                    .find_map(|part| part.trim().strip_prefix(proto).map(|s| s.to_string()))
            };
            pick("https=").or_else(|| pick("http=")).unwrap_or_default()
        } else {
            server.trim().to_string()
        };
        if addr.is_empty() {
            return None;
        }
        // reqwest/updater 需要带 scheme 的完整 URL
        if addr.starts_with("http://") || addr.starts_with("https://") {
            Some(addr)
        } else {
            Some(format!("http://{}", addr))
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(PtyManager::default())
        .manage(FsWatcher::default())
        .setup(|app| {
            #[cfg(target_os = "windows")]
            {
                use window_vibrancy::apply_acrylic;
                if let Some(window) = app.get_webview_window("main") {
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
            git_commit,
            git_diff,
            read_file,
            write_file,
            os_version,
            system_proxy,
            load_profiles,
            save_profiles,
            save_bg_image,
            load_bg_image,
            watch_dirs,
            debug_log_info,
            append_debug_log,
            clear_debug_log,
            create_entry,
            rename_entry,
            delete_entry
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Exit 是事件循环退出前的最后时机，覆盖所有退出路径（关窗口、托盘退出、
            // 更新后 relaunch），在这里统一回收 PTY 进程树
            if let tauri::RunEvent::Exit = event {
                kill_all_sessions(&app.state::<PtyManager>());
            }
        });
}

// ---------- 单元测试 ----------
// 只测纯函数：编码识别/往返、流式 UTF-8 解码、git 状态码归类、系统代理串解析。
// 这几处都是"错了不会崩、只会悄悄显示错东西"的地方，最值得钉住

#[cfg(test)]
mod tests {
    use super::*;

    // ----- 流式 UTF-8 解码 -----

    #[test]
    fn utf8_stream_handles_split_multibyte() {
        let s = "你好".as_bytes(); // 6 字节，从第 4 字节中间切开
        let mut leftover = Vec::new();
        assert_eq!(decode_utf8_stream(&mut leftover, &s[..4]), "你");
        assert_eq!(leftover.len(), 1); // 半个字符留着等下一块
        assert_eq!(decode_utf8_stream(&mut leftover, &s[4..]), "好");
        assert!(leftover.is_empty());
    }

    #[test]
    fn utf8_stream_skips_invalid_bytes_without_stalling() {
        let mut leftover = Vec::new();
        let out = decode_utf8_stream(&mut leftover, &[b'a', 0xFF, b'b']);
        assert_eq!(out, "a\u{FFFD}b");
        assert!(leftover.is_empty(), "非法字节不能永久滞留");
    }

    #[test]
    fn utf8_stream_never_accumulates_more_than_three_bytes() {
        let mut leftover = Vec::new();
        // 一个 4 字节 emoji 逐字节喂进去，中途 leftover 最多攒 3 字节
        let bytes = "🦀".as_bytes();
        let mut out = String::new();
        for b in bytes {
            out.push_str(&decode_utf8_stream(&mut leftover, &[*b]));
            assert!(leftover.len() <= 3);
        }
        assert_eq!(out, "🦀");
    }

    // ----- 编码识别与往返 -----

    const CN: &str = "老王在终端里敲下了一行命令，然后盯着输出发呆了整整三分钟。\
                      这段文本要足够长，编码嗅探才有足够的统计样本可用。";

    #[test]
    fn detects_plain_utf8() {
        let (text, enc) = decode_text(CN.as_bytes()).unwrap();
        assert_eq!(text, CN);
        assert_eq!(enc, "UTF-8");
    }

    #[test]
    fn detects_utf8_with_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(CN.as_bytes());
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, CN, "BOM 不能混进正文");
        assert_eq!(enc, "UTF-8-BOM");
    }

    #[test]
    fn detects_utf16le_despite_embedded_nul_bytes() {
        // PowerShell 5.1 的 Out-File 默认就是这个格式；ASCII 字符高位全是 0x00，
        // 二进制探测必须让位于 BOM 判断
        let mut bytes = vec![0xFF, 0xFE];
        for u in "hello 世界".encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, "hello 世界");
        assert_eq!(enc, "UTF-16LE");
    }

    #[test]
    fn detects_utf16be() {
        let mut bytes = vec![0xFE, 0xFF];
        for u in "hello 世界".encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, "hello 世界");
        assert_eq!(enc, "UTF-16BE");
    }

    #[test]
    fn detects_gbk_chinese_text() {
        let (bytes, _, err) = encoding_rs::GBK.encode(CN);
        assert!(!err, "测试数据本身应能用 GBK 表示");
        let (text, enc) = decode_text(&bytes).unwrap();
        assert_eq!(text, CN, "中文 GBK 文件不该被当成二进制或乱码");
        assert_ne!(enc, "UTF-8");
    }

    #[test]
    fn treats_nul_containing_data_as_binary() {
        assert!(decode_text(&[0x00, 0x01, 0x02, b'a']).is_none());
    }

    #[test]
    fn roundtrips_every_detected_encoding() {
        // 识别出来的编码必须能原样写回去，否则保存会静默转码
        for original in [
            {
                let mut v = vec![0xEF, 0xBB, 0xBF];
                v.extend_from_slice(CN.as_bytes());
                v
            },
            {
                let mut v = vec![0xFF, 0xFE];
                for u in CN.encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
                v
            },
            {
                let mut v = vec![0xFE, 0xFF];
                for u in CN.encode_utf16() {
                    v.extend_from_slice(&u.to_be_bytes());
                }
                v
            },
            encoding_rs::GBK.encode(CN).0.into_owned(),
            CN.as_bytes().to_vec(),
        ] {
            let (text, enc) = decode_text(&original).unwrap();
            let written =
                encode_text(&text, &enc).unwrap_or_else(|| panic!("编码 {} 无法写回", enc));
            let (again, enc2) = decode_text(&written).unwrap();
            assert_eq!(again, text, "编码 {} 往返后内容变了", enc);
            assert_eq!(enc2, enc, "编码 {} 往返后编码变了", enc);
        }
    }

    #[test]
    fn refuses_to_save_characters_the_original_encoding_cannot_hold() {
        // GBK 装不下 emoji：宁可报错，也不能用 '?' 悄悄替换掉用户的字符
        assert!(encode_text("🦀", "GBK").is_none());
        assert!(encode_text("🦀", "UTF-8").is_some());
        assert!(encode_text("🦀", ENC_UTF16LE).is_some());
    }

    #[test]
    fn unknown_encoding_label_falls_back_to_utf8() {
        assert_eq!(encode_text("abc", "no-such-encoding").unwrap(), b"abc");
        assert_eq!(encode_text("abc", "").unwrap(), b"abc");
    }

    // ----- 配置组密钥加解密 -----

    #[test]
    #[cfg(windows)]
    fn seals_and_unseals_secret() {
        let secret = "sk-ant-api03-中文也要能过-🦀";
        let sealed = seal(secret);
        assert!(sealed.starts_with(ENC_PREFIX), "落盘的必须是密文");
        assert!(!sealed.contains("sk-ant"), "密文里不能残留明文片段");
        assert_eq!(unseal(&sealed).as_deref(), Some(secret));
    }

    #[test]
    fn unseals_handwritten_plaintext_as_is() {
        // 用户直接手改配置文件写明文，也得能用
        assert_eq!(unseal("plain-token").as_deref(), Some("plain-token"));
    }

    #[test]
    fn refuses_to_unseal_corrupted_ciphertext() {
        // 换机器/换用户导致解不开时必须返回 None，绝不能把密文当明文注进环境变量
        assert_eq!(unseal(&format!("{}bm90LWEtcmVhbC1ibG9i", ENC_PREFIX)), None);
        assert_eq!(unseal(&format!("{}@@@not-base64@@@", ENC_PREFIX)), None);
    }

    #[test]
    #[ignore = "手动跑的性能测量，不进常规测试"]
    fn bench_detect_agent() {
        let t = std::time::Instant::now();
        let _ = detect_agent(std::process::id());
        println!("detect_agent 首次: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        for _ in 0..3 {
            let _ = detect_agent(std::process::id());
        }
        println!("后续 3 次平均: {:?}", t.elapsed() / 3);
    }

    // ----- 文件名校验 -----

    #[test]
    fn accepts_ordinary_file_names() {
        for name in ["a.txt", "组件.tsx", "my-file_2.rs", ".gitignore", "a.b.c"] {
            assert_eq!(invalid_file_name(name), None, "{} 应该是合法名字", name);
        }
    }

    #[test]
    fn rejects_windows_illegal_names() {
        assert!(invalid_file_name("").is_some());
        assert!(invalid_file_name("   ").is_some(), "全空格 trim 后为空");
        assert!(
            invalid_file_name("a/b").is_some(),
            "路径分隔符不能出现在名字里"
        );
        assert!(invalid_file_name("a\\b").is_some());
        assert!(invalid_file_name("a:b").is_some());
        assert!(invalid_file_name("a?").is_some());
        assert!(invalid_file_name("a*").is_some());
        assert!(
            invalid_file_name("name.").is_some(),
            "点结尾会被系统悄悄吞掉"
        );
        assert!(invalid_file_name("name ").is_none(), "尾随空格 trim 掉即可");
    }

    #[test]
    fn rejects_reserved_device_names_including_with_extension() {
        assert!(invalid_file_name("CON").is_some());
        assert!(invalid_file_name("nul").is_some(), "保留名不区分大小写");
        // CON.txt 一样打不开——保留名看的是第一段，不是整个文件名
        assert!(invalid_file_name("CON.txt").is_some());
        assert!(invalid_file_name("COM1.log").is_some());
        assert!(
            invalid_file_name("CONSOLE.txt").is_none(),
            "只是前缀相同不算"
        );
    }

    // ----- git 状态码归类 -----

    #[test]
    fn classifies_git_status_codes() {
        assert_eq!(classify("??"), "?");
        assert_eq!(classify("!!"), "!");
        assert_eq!(classify(" M"), "M");
        assert_eq!(classify("A "), "A");
        assert_eq!(classify("R "), "R");
        assert_eq!(classify(" D"), "D");
        // 删除优先级高于新增：AD = 加了又删了，按删除显示
        assert_eq!(classify("AD"), "D");
    }

    // ----- 重置时间归一化 -----

    #[test]
    fn normalizes_reset_timestamps() {
        assert_eq!(
            reset_to_ms(&serde_json::json!(1_700_000_000)),
            1_700_000_000_000
        );
        assert_eq!(
            reset_to_ms(&serde_json::json!("2026-01-01T00:00:00Z")),
            1_767_225_600_000
        );
        assert_eq!(reset_to_ms(&serde_json::json!(null)), 0);
        assert_eq!(reset_to_ms(&serde_json::json!("garbage")), 0);
    }
}
