use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

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

// 会话结束通知。以前只发一个 id，前端压根没监听，于是 shell 退出后
// 留给用户的是一个能打字、但永远不回话的黑框——没有提示，没有退出码，
// 也看不出到底是自己敲了 exit 还是进程崩了。
#[derive(Clone, Serialize)]
struct PtyExit {
    id: String,
    // 拿不到退出状态时为 None（进程被外部杀掉等）
    code: Option<u32>,
}

// pty_write 在会话已经不存在时回这个串。前端靠它区分「终端没了」和普通写入错误，
// 改动字面量会让前端的判断失效
const SESSION_GONE: &str = "session-gone";

// 输出聚合窗口。实测（见 bench_pty_read_chunks）ConPTY 平均每次只给 142 字节，
// 而读缓冲区有 4096——1MB 输出要读 8000 多次。照「读一次发一次」的老做法，
// 那就是 8000 个 IPC 事件、峰值每秒 8000+，每个还得序列化成 JSON。
// 攒够一帧再发，事件数降两个数量级，16ms 的延迟人眼分辨不出来。
const OUTPUT_FLUSH_MS: u64 = 16;

// 空闲时的巡检间隔。ConPTY 在客户端退出后**不会**让 read 返回 EOF
// （实测阻塞 12 秒仍不返回），所以「shell 退出了」这件事只能靠主动 try_wait 发现。
// 早先把 emit 放在读循环结束之后，那个事件其实永远发不出去
const EXIT_POLL_MS: u64 = 120;

// 各 shell 注入自己的 prompt，用 OSC 9;9 上报 cwd（供文件树联动）。结尾只用 \r。
// PowerShell 系：function prompt 里拼 OSC + 可见提示符
const POWERSHELL_INJECT: &str =
    "function prompt { $p=(Get-Location).Path; $e=[char]27; \"$e]9;9;$p$e\\PS $p> \" }; clear\r";
// CMD：PROMPT 里 $E=ESC、$P=当前路径、$G=>；先 cls 再设，避免回显那行注入命令
const CMD_INJECT: &str = "cls & prompt $E]9;9;$P$E\\$P$G \r";
// Git Bash：PROMPT_COMMAND 每次提示符前 printf 出 OSC 9;9；pwd -W 取 Windows 路径喂文件树
const BASH_INJECT: &str =
    "export PROMPT_COMMAND='printf \"\\033]9;9;%s\\033\\\\\" \"$(pwd -W 2>/dev/null || pwd)\"'\r";
// zsh：每次提示符前上报当前目录，供文件树联动。
const ZSH_INJECT: &str =
    "autoload -Uz add-zsh-hook; _brace_pwd_osc() { printf '\\033]9;9;%s\\033\\\\' \"$PWD\"; }; add-zsh-hook precmd _brace_pwd_osc; clear\r";

fn default_shell_path() -> String {
    #[cfg(target_os = "windows")]
    {
        "powershell.exe".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }
}

fn shell_type_from_path(path: &str) -> String {
    let name = Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.contains("powershell") || name == "pwsh.exe" || name == "pwsh" {
        "powershell".into()
    } else if name == "cmd.exe" || name == "cmd" {
        "cmd".into()
    } else if name.contains("zsh") {
        "zsh".into()
    } else if name.contains("bash") {
        "bash".into()
    } else {
        "sh".into()
    }
}

// 新建一个终端会话。shell_path 为空时使用系统默认 shell；shell_type 决定是否注入 cwd 上报。
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

    let use_default_shell = shell_path.trim().is_empty();
    let exe = if use_default_shell {
        default_shell_path()
    } else {
        shell_path
    };
    let effective_shell_type = if shell_type == "default" || use_default_shell {
        shell_type_from_path(&exe)
    } else {
        shell_type
    };
    let mut cmd = CommandBuilder::new(&exe);
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
    let inject = match effective_shell_type.as_str() {
        "powershell" => POWERSHELL_INJECT,
        "cmd" => CMD_INJECT,
        "bash" => BASH_INJECT,
        "zsh" => ZSH_INJECT,
        _ => "",
    };
    if !inject.is_empty() {
        let _ = writer.write_all(inject.as_bytes());
        let _ = writer.flush();
    }

    // 读和发拆成两个线程。
    //
    // 拆开的原因不是为了好看，是因为这条路上有两件事不能放在一起做：
    //   1. ConPTY 平均每次只给一百多字节，读一次发一次会把 IPC 打爆（见 C3 的实测）；
    //   2. ConPTY 在 shell 退出后不会让 read 返回 EOF，read 会一直阻塞。
    //      把「进程退出了」的判断挂在读循环结束之后，那个判断永远等不到。
    //
    // 所以：读线程只负责把字节搬进缓冲区；发送线程按帧把缓冲区清空，
    // 顺便周期性地 try_wait 看看进程还在不在。
    let pending = Arc::new((Mutex::new(String::new()), Condvar::new()));

    let pending_writer = Arc::clone(&pending);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut leftover: Vec<u8> = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = decode_utf8_stream(&mut leftover, &buf[..n]);
                    if data.is_empty() {
                        continue;
                    }
                    let (lock, cv) = &*pending_writer;
                    match lock.lock() {
                        Ok(mut g) => g.push_str(&data),
                        Err(_) => break,
                    }
                    cv.notify_one();
                }
                Err(_) => break,
            }
        }
        // 走到这儿说明 master 已经被 drop 了（发送线程摘掉 session 时会发生），
        // 这正是唯一能把卡住的 read 唤醒的办法
    });

    let app_handle = app.clone();
    let sid = id.clone();
    let child_for_wait = Arc::clone(&child);
    std::thread::spawn(move || {
        let (lock, cv) = &*pending;

        // 把缓冲区里攒的内容一次性发给前端
        let flush = |app: &AppHandle, id: &str| {
            let chunk = match lock.lock() {
                Ok(mut g) => std::mem::take(&mut *g),
                Err(_) => String::new(),
            };
            if !chunk.is_empty() {
                let _ = app.emit(
                    "pty-output",
                    PtyOutput {
                        id: id.to_string(),
                        data: chunk,
                    },
                );
            }
        };

        loop {
            // 有数据就攒一帧再发；没有就睡一会，醒来查一次进程状态。
            // 空闲的终端在这里是完全阻塞的，不烧 CPU
            let has_data = match lock.lock() {
                Ok(g) if !g.is_empty() => true,
                Ok(g) => {
                    let waited = cv
                        .wait_timeout(g, std::time::Duration::from_millis(EXIT_POLL_MS))
                        .map(|(g, _)| !g.is_empty());
                    waited.unwrap_or(false)
                }
                Err(_) => break,
            };

            if has_data {
                // 松开锁睡一帧，让这段时间内到达的输出一起并进来
                std::thread::sleep(std::time::Duration::from_millis(OUTPUT_FLUSH_MS));
                flush(&app_handle, &sid);
            }

            // shell 退出了吗？只能主动问，read 那边永远不会告诉我们。
            //
            // 必须是 try_wait 而不是 wait：wait 会攥着这把锁一直阻塞到进程结束，
            // 而 pty_close 要拿同一把锁去 kill——那就是个死锁。改成非阻塞轮询后
            // 持锁时间只有一瞬，顺带把这个隐患也消掉了
            let status = child_for_wait
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten());
            let Some(status) = status else {
                continue;
            };

            // 退出前把最后一点残留发完，别让用户少看见最后几行
            flush(&app_handle, &sid);

            // 摘掉 session。两个作用：pty_write 从此能明确回「会话不存在」，
            // 而不是把用户敲的字灌进一个死管道；同时 PtySession 析构会 drop master，
            // 卡在 read 上的读线程随之解除阻塞退出——否则每关一个标签漏一个线程
            if let Some(mgr) = app_handle.try_state::<PtyManager>() {
                if let Ok(mut sessions) = mgr.sessions.lock() {
                    sessions.remove(&sid);
                }
            }

            // 上面那次 flush 和进程退出之间有个窄窗口：读线程可能刚好又塞进了
            // 最后一批字节。等一帧再兜一次，否则 shell 退出前的最后几行会静静消失
            std::thread::sleep(std::time::Duration::from_millis(OUTPUT_FLUSH_MS));
            flush(&app_handle, &sid);

            let _ = app_handle.emit(
                "pty-exit",
                PtyExit {
                    id: sid.clone(),
                    code: Some(status.exit_code()),
                },
            );
            break;
        }
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
    // 会话不在了：进程已退出，或者标签刚被关掉。以前这里直接返回 Ok，
    // 前后端一起假装写成功了——用户敲的每个字都进了黑洞，还没有任何提示
    let Some(s) = sessions.get_mut(&id) else {
        return Err(SESSION_GONE.into());
    };
    s.writer
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    s.writer.flush().map_err(|e| e.to_string())?;
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

    // 凭据交互必须关掉。这些子进程带 CREATE_NO_WINDOW 启动、stdin 是空的，
    // git 一旦决定「问用户要密码」就会永远等在那里，而 output() 没有超时——
    // 表现是 GitPanel 整个卡死，转圈转到用户杀进程为止，日志里什么都没有。
    // 关掉之后同样的场景会立刻失败并带上原因，前端据此引导用户先去终端里
    // 跑一次 git push 完成认证。快速失败比静默死锁好得多。
    //
    // 故意不动 GIT_ASKPASS：用户可能配了自己的凭据助手，覆盖它等于砸掉一条
    // 本来能正常工作的认证路径。上面两个已经堵住会死锁的那条。
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GCM_INTERACTIVE", "never");

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

// include_ignored 由前端的「Git 装饰」开关决定。
//
// --ignored 会让 git 把所有被忽略的文件逐条列出来——在带 node_modules 或 target
// 的仓库里就是几万条路径，序列化一遍再走一趟 IPC，每 20 秒一次。而这些数据
// 只有文件树的装饰用得上：装饰关掉时状态栏只要分支名，那笔开销纯属白烧。
#[tauri::command]
fn git_status(cwd: String, include_ignored: bool) -> GitStatus {
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
    let mut status_args: Vec<&str> = vec!["status", "--porcelain", "-z"];
    if include_ignored {
        status_args.push("--ignored");
    }
    if let Some(out) = run_git(&cwd, &status_args) {
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

// 提交的结果。
//
// 「提交成功但推送失败」必须能和「整体失败」区分开：网络断了、没有 upstream、
// 认证过期都会让 push 挂掉，而这时改动已经实实在在提交到本地了。以前这里直接
// 把 push 的错误往外抛，前端只显示「失败」，用户的第一反应是再点一次——
// 于是要么撞上 nothing to commit，要么多出一个空提交。报错报得不准，
// 就是在诱导用户破坏自己的提交历史。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommitOutcome {
    committed: bool,
    pushed: bool,
    // Some 表示提交已完成、推送失败，内容是 git 给的原因
    push_error: Option<String>,
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
) -> Result<CommitOutcome, String> {
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

    // 走到这里 commit 一定成功了（失败的话上面已经 ? 出去了）。
    // 所以 push 的错误绝不能用 ? ——那会把「已提交」这个事实一起丢掉
    if push {
        match run_git_out(&cwd, &["push"], false) {
            Ok(_) => Ok(CommitOutcome {
                committed: true,
                pushed: true,
                push_error: None,
            }),
            Err(e) => Ok(CommitOutcome {
                committed: true,
                pushed: false,
                push_error: Some(e),
            }),
        }
    } else {
        Ok(CommitOutcome {
            committed: true,
            pushed: false,
            push_error: None,
        })
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
    shell_type: String, // powershell | cmd | bash | zsh | sh
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

fn push_shell(shells: &mut Vec<ShellInfo>, id: &str, name: &str, path: String, shell_type: &str) {
    if shells.iter().any(|s| s.path == path) {
        return;
    }
    shells.push(ShellInfo {
        id: id.into(),
        name: name.into(),
        path,
        shell_type: shell_type.into(),
    });
}

// 检测系统里可用的 shell
#[tauri::command]
fn detect_shells() -> Vec<ShellInfo> {
    let mut shells = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Some(p) = which("powershell.exe") {
            push_shell(
                &mut shells,
                "powershell",
                "Windows PowerShell",
                p,
                "powershell",
            );
        }
        if let Some(p) = which("pwsh.exe") {
            push_shell(&mut shells, "pwsh", "PowerShell 7", p, "powershell");
        }
        if let Some(p) = which("cmd.exe") {
            push_shell(&mut shells, "cmd", "Command Prompt", p, "cmd");
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
            push_shell(&mut shells, "gitbash", "Git Bash", bp, "bash");
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(shell) = std::env::var("SHELL") {
            let shell_type = shell_type_from_path(&shell);
            let name = match shell_type.as_str() {
                "zsh" => "Zsh",
                "bash" => "Bash",
                _ => "Login Shell",
            };
            push_shell(&mut shells, "default", name, shell, &shell_type);
        }
        for (id, name, path, shell_type) in [
            ("zsh", "Zsh", "/bin/zsh", "zsh"),
            ("bash", "Bash", "/bin/bash", "bash"),
            ("sh", "sh", "/bin/sh", "sh"),
        ] {
            if Path::new(path).is_file() {
                push_shell(&mut shells, id, name, path.to_string(), shell_type);
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
    // 上下文占用是不是真读到了。
    // 缓存里缺 context_window.used_percentage 时 context_pct 会是 0.0，
    // 但那含义是「不知道」，不是「用了 0%」——以前两者在前端长得一模一样，
    // 于是就有了「用量偶尔显示 0%」这种查无实据的现象
    has_context: bool,
    // 数据不正常时说明原因，空串表示一切正常。给前端做 tooltip，
    // 也是下次再撞见异常时唯一的线索
    note: String,
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
                None => {
                    stats.note = "cache-missing".into();
                    return stats;
                }
            };
            // 新鲜度：缓存超过 15 分钟没更新（claude 没在活跃跑，或开关已关脚本停写），
            // 不拿旧数据糊弄——直接返回，前端隐藏整条
            let age = cache["updated_at"]
                .as_f64()
                .map_or(f64::INFINITY, |u| Utc::now().timestamp() as f64 - u);
            if age > 900.0 {
                stats.note = "cache-stale".into();
                return stats;
            }
            stats.has_data = true;
            if let Some(m) = cache["model"]["display_name"]
                .as_str()
                .or_else(|| cache["model"]["id"].as_str())
            {
                stats.model = m.to_string();
            }
            // 缺字段时 as_f64() 给的是 None。以前这里 unwrap_or(0.0)，
            // 把「读不到」直接说成「用了 0%」——用户看到的就是那个莫名其妙的 0%
            match cache["context_window"]["used_percentage"].as_f64() {
                Some(v) => {
                    stats.context_pct = v;
                    stats.has_context = true;
                }
                None => stats.note = "context-missing".into(),
            }
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
                stats.has_context = true;
                stats.model = "Codex".into();
                stats.context_pct = ctx;
                stats.codex_total_tokens = total;
            } else {
                // 找不到会话记录：CODEX_HOME 指到别处，或者这个会话还没写过 token_count
                stats.note = "codex-session-missing".into();
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

// 背景图 data URL 的体积上限。base64 会把原图撑大约 1/3，8MB 差不多对应
// 一张 6MB 的照片——当壁纸绰绰有余。这里以前完全不校验，于是一张 20MB 的图
// 会变成 27MB 的字符串：过一趟 IPC、原样落盘、每次启动再整个读进内存，
// 最后塞进 CSS 的 url()。
const MAX_BG_BYTES: usize = 8 * 1024 * 1024;

// data_url 传空串 = 清除背景
#[tauri::command]
fn save_bg_image(app: AppHandle, data_url: String) -> Result<(), String> {
    if data_url.len() > MAX_BG_BYTES {
        return Err(format!(
            "图片太大：{:.1} MB，上限 {} MB",
            data_url.len() as f64 / 1024.0 / 1024.0,
            MAX_BG_BYTES / 1024 / 1024
        ));
    }
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

// DPAPI 到底能不能用，只有真跑一次加解密往返才知道。
//
// 这里以前写的是 cfg!(windows)——编译期常量，在 Windows 上恒为 true。
// 而 seal() 遇到 DPAPI 失败会静默退回明文。两者一叠加，结果是 token 明文
// 躺在磁盘上、界面却告诉用户「已用 DPAPI 加密」。用户会因为这句话放心地去
// 同步那个配置文件。安全提示撒谎比根本没有提示更危险。
//
// 加密能力在进程生命周期内不会变，探一次缓存住即可。
fn encryption_works() -> bool {
    static PROBE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROBE.get_or_init(|| {
        const SAMPLE: &[u8] = b"brace-dpapi-probe";
        match dpapi(SAMPLE, true) {
            // 加得上还要解得开才算数：只成功一半的加密等于没有加密
            Some(blob) => dpapi(&blob, false).as_deref() == Some(SAMPLE),
            None => false,
        }
    })
}

// 明文 → 落盘形态。加密不可用时退回明文，不阻塞用户使用——
// 但此时 encryption_works() 会返回 false，界面必须如实显示「未加密」
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
    // None = 前端没动过这一项，沿用已存的值；Some("") = 明确清空。
    //
    // 以前这里是 String，空串同时背着「没改」和「清空」两个意思，于是
    // 「改个变量名再改回来」会走出这么一条路：前端把这行标成「未设置」，
    // 保存时传空串，后端却按 (配置组, 变量名) 查到了旧密文并原样留下——
    // 界面说没有，磁盘上有，而且还在往新终端里注入。
    value: Option<String>,
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
        encryption_available: encryption_works(),
    }
}

// 决定一个 secret 变量最终落盘成什么。
//
// incoming 三种取值对应三种意图，混淆任意两个都会出事：
//   None       前端没动过这一项 → 沿用已存的密文（前端本来也拿不到明文）
//   Some("")   明确清空，或者改了变量名 → 旧密文作废
//   Some(值)   填了新值 → 加密后落盘
//
// 单独拎出来是为了能直接测：之前「改名再改回原名」就是栽在这段逻辑上——
// 界面显示「未设置」，磁盘上旧密文却还在，而且继续注入新终端
fn resolve_secret_value(incoming: Option<String>, existing: Option<&String>) -> String {
    match incoming {
        None => existing.cloned().unwrap_or_default(),
        Some(s) if s.is_empty() => String::new(),
        Some(s) => seal(&s),
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
                        v.value.unwrap_or_default()
                    } else {
                        resolve_secret_value(v.value, kept.get(&(p.id.clone(), v.key.clone())))
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

    // 这条锁的是整个加密提示的可信度：界面上说「已加密」，磁盘上就必须真的是密文。
    // 以前 encryption_available 用的是编译期的 cfg!(windows)，而 seal() 在 DPAPI
    // 失败时会静默退回明文——两者一脱节，用户看到的就是一句谎话。
    // 不分平台跑：非 Windows 上 dpapi() 恒为 None，两边都该是 false。
    #[test]
    fn encryption_flag_matches_what_seal_actually_does() {
        let sealed = seal("probe-value");
        assert_eq!(
            encryption_works(),
            sealed.starts_with(ENC_PREFIX),
            "encryption_works() 报的状态和 seal() 的实际行为对不上"
        );
    }

    // ----- secret 的三态语义 -----

    #[test]
    fn keeps_existing_secret_when_frontend_sends_nothing() {
        // 前端拿不到明文，所以「没改」只能用 None 表达
        let old = "enc:AAAA".to_string();
        assert_eq!(resolve_secret_value(None, Some(&old)), old);
    }

    #[test]
    fn yields_empty_when_nothing_sent_and_nothing_stored() {
        assert_eq!(resolve_secret_value(None, None), "");
    }

    // 这条是「改名再改回原名」那个 bug 的回归测试：前端明确送来空串时，
    // 哪怕旧密文还躺在 kept 里也必须丢掉。否则界面说「未设置」，
    // 磁盘上的旧密钥却继续往新终端里注入
    #[test]
    fn clears_secret_when_frontend_explicitly_sends_empty() {
        let old = "enc:AAAA".to_string();
        assert_eq!(resolve_secret_value(Some(String::new()), Some(&old)), "");
    }

    #[test]
    fn seals_newly_provided_secret() {
        let out = resolve_secret_value(Some("sk-new-token".into()), None);
        if encryption_works() {
            assert!(out.starts_with(ENC_PREFIX), "落盘的必须是密文");
            assert!(!out.contains("sk-new-token"), "密文里不能残留明文片段");
        } else {
            // 加密不可用时 seal 有意退回明文（不阻塞用户），
            // 而 encryption_works() 会把这件事如实告诉界面——见上面那条一致性测试
            assert_eq!(out, "sk-new-token");
        }
    }

    #[test]
    fn new_value_wins_over_stored_one() {
        let old = "enc:OLD".to_string();
        let out = resolve_secret_value(Some("fresh".into()), Some(&old));
        assert_ne!(out, old);
    }

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

    // PTY 读循环的实测数据。回答一个问题：现在「每次 read 就 emit 一个事件」的做法，
    // 在高频输出下到底会产生多少个事件？
    //
    // 关键不是总字节数，是**平均块大小**——ConPTY 每次给多少字节，直接决定事件数量。
    // 如果块普遍接近 4096，那事件数就是可接受的；如果只有几百字节，
    // 那么一次 cargo build 的输出就能刷出几万个 IPC 事件，每个都要序列化成 JSON。
    //
    // 跑法：cargo test --manifest-path src-tauri/Cargo.toml --lib -- --ignored --nocapture bench_pty
    #[test]
    #[ignore = "手动跑的性能测量，不进常规测试"]
    #[cfg(windows)]
    fn bench_pty_read_chunks() {
        let dir = std::env::temp_dir().join("brace-bench");
        std::fs::create_dir_all(&dir).expect("建临时目录");
        let file = dir.join("big.txt");
        // 约 10MB 文本，行长 120 —— 贴近编译日志那种输出形态
        // 约 1MB。块大小分布不需要靠总量堆出来，采样够了就行——
        // 第一版用了 10MB，ConPTY 要对每一行做完整的终端仿真，跑了十分钟还没完
        let filler = "x".repeat(110);
        let content: String = (0..8_000).map(|i| format!("{i:06} {filler}\n")).collect();
        std::fs::write(&file, &content).expect("写测试数据");

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("开 pty");
        let mut cmd = CommandBuilder::new("cmd.exe");
        cmd.args(["/c", "type", file.to_str().unwrap()]);
        let mut child = pair.slave.spawn_command(cmd).expect("起进程");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("拿 reader");

        // 读循环放到独立线程里，主线程用「多久没有新数据」来判断输出结束。
        // 不能在循环里做超时判断——read() 一旦阻塞，循环体根本不会执行到那一行。
        // 这个结构顺便回答了另一个关键问题：客户端退出后 read 到底给不给 EOF。
        // 生产代码里 pty-exit 事件正是在读循环结束之后才发的，如果永远拿不到 EOF，
        // 那个事件就永远发不出去，A1 的退出提示也就无从谈起
        let (tx, rx) = std::sync::mpsc::channel::<(usize, std::time::Instant)>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send((n, std::time::Instant::now())).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // tx 在这里析构，主线程会收到 Disconnected —— 那才代表真的读到了尽头
        });

        let start = std::time::Instant::now();
        let mut sizes: Vec<usize> = Vec::new();
        let mut total = 0u64;
        let mut last_data = std::time::Instant::now();
        let mut stamps: Vec<std::time::Instant> = Vec::new();
        let got_eof;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(12)) {
                Ok((n, at)) => {
                    sizes.push(n);
                    stamps.push(at);
                    total += n as u64;
                    last_data = std::time::Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    got_eof = true;
                    break;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    got_eof = false;
                    break;
                }
            }
        }
        // 用最后一次收到数据的时刻算，别把末尾那段空等算进吞吐
        let elapsed = last_data.duration_since(start);

        // 拿不到 EOF 的话，接着验证一件事：主动关掉 master 能不能把卡住的 read 唤醒。
        // 这直接决定修复方案——能唤醒就让「等进程退出」的线程去摘 session（连带 drop master），
        // 读线程自己就收摊了；唤不醒的话每个关掉的标签都会漏一个永久阻塞的线程
        let freed_by_drop = if got_eof {
            None
        } else {
            drop(pair.master);
            Some(matches!(
                rx.recv_timeout(std::time::Duration::from_secs(5)),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            ))
        };

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&file);

        if let Some(freed) = freed_by_drop {
            println!(
                "drop(master) 能否唤醒卡住的 read：{}",
                if freed {
                    "能 —— 摘掉 session 就足以让读线程退出"
                } else {
                    "不能 —— 读线程会永久泄漏，需要另想办法"
                }
            );
        }

        println!(
            "客户端退出后是否收到 EOF：{}",
            if got_eof {
                "是 —— 读循环能正常结束，pty-exit 发得出去"
            } else {
                "否 —— read() 一直阻塞，pty-exit 永远发不出来（A1 会失效）"
            }
        );

        sizes.sort_unstable();
        let reads = sizes.len();
        let avg = (total as usize).checked_div(reads).unwrap_or(0);
        let median = sizes.get(reads / 2).copied().unwrap_or(0);
        let secs = elapsed.as_secs_f64();

        println!("--- PTY 读循环实测 ---");
        println!(
            "总字节      : {} ({:.1} MB)",
            total,
            total as f64 / 1048576.0
        );
        println!("read 次数   : {reads}  ← 当前实现下等量的 IPC 事件数");
        println!("平均块大小  : {avg} 字节（读缓冲区 4096）");
        println!("中位块大小  : {median} 字节");
        println!(
            "最小 / 最大 : {} / {}",
            sizes.first().unwrap_or(&0),
            sizes.last().unwrap_or(&0)
        );
        println!("耗时        : {:.2}s", secs);
        if secs > 0.0 {
            println!("事件速率    : {:.0} 次/秒", reads as f64 / secs);
        }

        // 按 OUTPUT_FLUSH_MS 的窗口模拟一遍聚合，看事件数能降到什么量级。
        // 和真实实现（有数据 → 睡一帧 → 一次性发）不完全等价，但数量级是对的
        let mut aggregated = 0usize;
        let mut window: Option<std::time::Instant> = None;
        for at in &stamps {
            match window {
                Some(w) if at.duration_since(w).as_millis() < u128::from(OUTPUT_FLUSH_MS) => {}
                _ => {
                    aggregated += 1;
                    window = Some(*at);
                }
            }
        }
        println!("--- 按 {OUTPUT_FLUSH_MS}ms 窗口聚合后 ---");
        println!("事件数      : {aggregated}（原本 {reads}）");
        if aggregated > 0 {
            println!(
                "降幅        : {:.1}%",
                (1.0 - aggregated as f64 / reads as f64) * 100.0
            );
            println!("平均每事件  : {} 字节", total as usize / aggregated);
        }
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
