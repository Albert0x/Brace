use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::profiles::active_env;

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
pub(crate) struct PtyManager {
    sessions: Mutex<HashMap<String, PtySession>>,
}

impl PtyManager {
    // 用量统计只需要知道某个会话的进程 id（用来往下找 claude/codex 进程），
    // 没必要把整张 sessions 表暴露出去。拆模块之前这两段代码在同一个文件里，
    // 直接摸字段不花成本，也就一直没人意识到这是个耦合
    pub(crate) fn pid_of(&self, id: &str) -> Option<u32> {
        self.sessions.lock().ok()?.get(id)?.pid
    }
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
pub(crate) fn pty_create(
    app: AppHandle,
    manager: State<'_, PtyManager>,
    id: String,
    rows: u16,
    cols: u16,
    cwd: String,
    shell_path: String,
    shell_type: String,
    // 额外的命令行参数。SSH 会话靠它把 -p / -i / user@host 传给 ssh.exe；
    // 普通 shell 传空数组。以后的自定义 shell（WSL 等）走同一条路
    args: Vec<String>,
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

    // 说是 SSH 却没给客户端路径，意味着系统里没找到 ssh.exe。这里必须明确失败：
    // 否则下面会把它当成「用默认 shell」，于是开出一个本地 PowerShell，
    // 而用户以为自己连上了远程主机——那比连不上糟糕得多
    if shell_type == "ssh" && shell_path.trim().is_empty() {
        return Err("找不到 ssh 客户端（需要 Windows 自带的 OpenSSH）".into());
    }

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
    // SSH 会话的 -p / -i / user@host 从这里进去；普通 shell 是空的
    if !args.is_empty() {
        cmd.args(&args);
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
pub(crate) fn pty_write(
    manager: State<'_, PtyManager>,
    id: String,
    data: String,
) -> Result<(), String> {
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
pub(crate) fn pty_resize(
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
pub(crate) fn pty_close(manager: State<'_, PtyManager>, id: String) -> Result<(), String> {
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
pub(crate) fn kill_all_sessions(manager: &PtyManager) {
    let sessions = match manager.sessions.lock() {
        Ok(mut s) => std::mem::take(&mut *s),
        Err(e) => std::mem::take(&mut *e.into_inner()), // 有线程 panic 过也照样收尸
    };
    for (_, session) in sessions {
        kill_session(&session);
    }
}

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

    // ----- PTY 退出检测 -----

    // A1 的核心机制测试：shell 退出后，靠 try_wait 轮询能不能拿到退出码。
    //
    // 为什么非要测这个：ConPTY 在客户端退出后不会让 read 返回 EOF
    // （见 bench_pty_read_chunks 的实测），所以「读循环结束了 → 进程没了」
    // 这条推断根本不成立。整个退出提示和原地重启都建立在轮询能奏效之上,
    // 这一条要是坏了，用户看到的又会是那个能打字却不回话的黑框。
    #[test]
    #[cfg(windows)]
    fn detects_shell_exit_by_polling_not_by_eof() {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("开 pty");

        let mut cmd = CommandBuilder::new("cmd.exe");
        // 立刻以 3 退出，退出码要能原样传回前端
        cmd.args(["/c", "exit", "3"]);
        let child = pair.slave.spawn_command(cmd).expect("起进程");
        drop(pair.slave);

        let child: Arc<Mutex<Box<dyn Child + Send + Sync>>> = Arc::new(Mutex::new(child));

        // 完全按生产代码里发送线程的做法轮询
        let start = std::time::Instant::now();
        let mut code = None;
        while start.elapsed() < std::time::Duration::from_secs(10) {
            let status = child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten());
            if let Some(status) = status {
                code = Some(status.exit_code());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(EXIT_POLL_MS));
        }

        assert_eq!(
            code,
            Some(3),
            "try_wait 必须能发现进程退出并带回退出码；拿不到就说明 pty-exit 发不出去"
        );
    }
}
