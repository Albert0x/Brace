// 命令历史：跨标签、可搜索、可收藏，纯本地。
//
// 数据来源只有一个：shell 集成脚本（src-tauri/shell/）在每次提示符前上报的
//   OSC 6973;cmd;<nonce>;<退出码>;<百分号编码的命令>
// 前端只负责把这段原样转过来，校验、解析、过滤都在这里做。
//
// nonce 的意义：终端里任何程序都能打印这段序列——cat 一个文件、SSH 远端的输出——
// 没有它，谁都能往历史里塞一条假命令。nonce 每个会话一个，只经环境变量交给 shell，
// 屏幕上的内容拿不到它。

use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};

use crate::redact::looks_secret;

// 上限只算非收藏条目。收藏是用户亲手留下的，不能因为最近敲得多就被挤掉
const MAX_ENTRIES: usize = 2000;
// 超长的通常是粘贴进来的脚本，不是「一条命令」，存了也没法在列表里看
const MAX_COMMAND_LEN: usize = 4000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    pub id: u64,
    pub command: String,
    pub exit_code: i32,
    pub cwd: String,
    pub shell_type: String,
    // unix 毫秒
    pub at: i64,
    #[serde(default)]
    pub favorite: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct Store {
    next_id: u64,
    entries: Vec<Entry>,
}

// 内存里放一份，所有标签共用。不加锁的话两个标签同时执行命令，
// 各自「读文件 → 追加 → 写回」，后写的会把先写的那条冲掉
#[derive(Default)]
pub(crate) struct HistoryState {
    store: Mutex<Option<Store>>,
}

pub(crate) fn new_nonce() -> String {
    // RandomState 的种子来自操作系统随机源。两个独立实例拼出 128 位，
    // 只要求「屏幕上的程序猜不到」，不需要密码学强度的随机数库
    let a = RandomState::new().hash_one(std::process::id());
    let b = RandomState::new().hash_one(std::time::SystemTime::now());
    format!("{:016x}{:016x}", a, b)
}

#[derive(Debug, PartialEq)]
struct Reported {
    exit_code: i32,
    command: String,
}

// 解析 OSC 6973 的内容（不含前面的 "6973;"）。nonce 不对、格式不对，一律当没看见
fn parse_report(payload: &str, nonce: &str) -> Option<Reported> {
    let rest = payload.strip_prefix("cmd;")?;
    let (got_nonce, rest) = rest.split_once(';')?;
    if nonce.is_empty() || got_nonce != nonce {
        return None;
    }
    let (code, encoded) = rest.split_once(';')?;
    let exit_code = code.trim().parse().ok()?;
    Some(Reported {
        exit_code,
        command: percent_decode(encoded)?,
    })
}

// 脚本只编码了 % 和控制字符，其余原样（包括中文）。
// 按字节解码再转 UTF-8：%XX 拼出来的必须是合法 UTF-8，否则整条丢掉
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl Store {
    // 返回 true 表示有变化、需要落盘
    fn add(&mut self, command: &str, exit_code: i32, cwd: &str, shell_type: &str, at: i64) -> bool {
        let command = command.trim();
        if command.is_empty() || command.len() > MAX_COMMAND_LEN || looks_secret(command) {
            return false;
        }
        // 和上一条完全相同就只刷新时间和结果，不重复堆一条——
        // 连着敲三次 git status 不该在列表里占三行
        if let Some(last) = self.entries.last_mut() {
            if last.command == command {
                last.exit_code = exit_code;
                last.cwd = cwd.to_string();
                last.at = at;
                return true;
            }
        }
        self.next_id += 1;
        self.entries.push(Entry {
            id: self.next_id,
            command: command.to_string(),
            exit_code,
            cwd: cwd.to_string(),
            shell_type: shell_type.to_string(),
            at,
            favorite: false,
        });
        self.trim();
        true
    }

    // 从最旧的非收藏条目开始删，直到非收藏条目不超过上限
    fn trim(&mut self) {
        let plain = self.entries.iter().filter(|e| !e.favorite).count();
        let mut excess = plain.saturating_sub(MAX_ENTRIES);
        if excess == 0 {
            return;
        }
        self.entries.retain(|e| {
            if excess > 0 && !e.favorite {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到配置目录：{}", e))?;
    Ok(dir.join("history.json"))
}

fn load(app: &AppHandle) -> Store {
    // 文件坏了就从空开始，而不是让整个功能报错——历史丢了可惜，但不该拦住用户用终端
    history_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, store: &Store) -> Result<(), String> {
    let path = history_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 临时文件 + rename，写一半崩了也不会留下半个损坏的文件
    let text = serde_json::to_string(store).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

// 拿到内存里那份（第一次用时从磁盘读），在锁里改，改完落盘
fn with_store<T>(
    app: &AppHandle,
    state: &HistoryState,
    f: impl FnOnce(&mut Store) -> (T, bool),
) -> Result<T, String> {
    let mut guard = state.store.lock().map_err(|e| e.to_string())?;
    let store = guard.get_or_insert_with(|| load(app));
    let (out, changed) = f(store);
    if changed {
        save(app, store)?;
    }
    Ok(out)
}

// 前端收到 OSC 6973 后把内容原样转过来。返回是否记下了一条
#[tauri::command]
pub(crate) fn history_report(
    app: AppHandle,
    state: State<'_, HistoryState>,
    pty: State<'_, crate::pty::PtyManager>,
    session_id: String,
    payload: String,
    cwd: String,
) -> Result<bool, String> {
    let Some((nonce, shell_type)) = pty.history_nonce_of(&session_id) else {
        return Ok(false);
    };
    let Some(r) = parse_report(&payload, &nonce) else {
        return Ok(false);
    };
    let at = chrono::Utc::now().timestamp_millis();
    with_store(&app, &state, |s| {
        let added = s.add(&r.command, r.exit_code, &cwd, &shell_type, at);
        (added, added)
    })
}

// 新的在前
#[tauri::command]
pub(crate) fn history_list(
    app: AppHandle,
    state: State<'_, HistoryState>,
) -> Result<Vec<Entry>, String> {
    with_store(&app, &state, |s| {
        (s.entries.iter().rev().cloned().collect(), false)
    })
}

#[tauri::command]
pub(crate) fn history_set_favorite(
    app: AppHandle,
    state: State<'_, HistoryState>,
    id: u64,
    favorite: bool,
) -> Result<(), String> {
    with_store(&app, &state, |s| {
        let hit = s.entries.iter_mut().find(|e| e.id == id);
        let changed = hit.map(|e| e.favorite = favorite).is_some();
        if changed && !favorite {
            s.trim();
        }
        ((), changed)
    })
}

#[tauri::command]
pub(crate) fn history_delete(
    app: AppHandle,
    state: State<'_, HistoryState>,
    id: u64,
) -> Result<(), String> {
    with_store(&app, &state, |s| {
        let before = s.entries.len();
        s.entries.retain(|e| e.id != id);
        ((), s.entries.len() != before)
    })
}

// 清空保留收藏：「清空历史」清的是流水账，不是用户亲手收藏的东西
#[tauri::command]
pub(crate) fn history_clear(app: AppHandle, state: State<'_, HistoryState>) -> Result<(), String> {
    with_store(&app, &state, |s| {
        s.entries.retain(|e| e.favorite);
        ((), true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn parses_a_report() {
        let r = parse_report(&format!("cmd;{N};0;git status"), N).unwrap();
        assert_eq!(
            r,
            Reported {
                exit_code: 0,
                command: "git status".into()
            }
        );
    }

    #[test]
    fn decodes_what_the_scripts_encode() {
        // 脚本会编码 % 和控制字符；分号、引号、中文都原样
        let r = parse_report(
            &format!("cmd;{N};3;echo \"a%0Ab\"; printf '50%25\t'%09x %1B %07 中文"),
            N,
        )
        .unwrap();
        assert_eq!(r.exit_code, 3);
        assert_eq!(r.command, "echo \"a\nb\"; printf '50%\t'\tx \x1b \x07 中文");
    }

    #[test]
    fn negative_exit_code() {
        assert_eq!(
            parse_report(&format!("cmd;{N};-1;x"), N).unwrap().exit_code,
            -1
        );
    }

    #[test]
    fn rejects_wrong_or_missing_nonce() {
        assert_eq!(parse_report(&format!("cmd;{N};0;ls"), "other"), None);
        assert_eq!(parse_report("cmd;;0;ls", ""), None);
        // 屏幕上的程序伪造时最可能的样子：根本不知道有 nonce 这一段
        assert_eq!(parse_report("cmd;0;ls", N), None);
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(parse_report(&format!("cwd;{N};0;ls"), N), None);
        assert_eq!(parse_report(&format!("cmd;{N};zero;ls"), N), None);
        assert_eq!(parse_report(&format!("cmd;{N};0;bad %G1"), N), None);
        assert_eq!(parse_report(&format!("cmd;{N};0;cut %4"), N), None);
        // %XX 拼出非法 UTF-8
        assert_eq!(parse_report(&format!("cmd;{N};0;%FF"), N), None);
    }

    fn add(s: &mut Store, cmd: &str) -> bool {
        s.add(cmd, 0, "C:/", "bash", 1)
    }

    #[test]
    fn skips_empty_secret_and_huge() {
        let mut s = Store::default();
        assert!(!add(&mut s, "   "));
        assert!(!add(
            &mut s,
            "export ANTHROPIC_AUTH_TOKEN=sk-ant-abcdefghijklmnopq"
        ));
        assert!(!add(&mut s, &"x".repeat(MAX_COMMAND_LEN + 1)));
        assert!(s.entries.is_empty());
    }

    #[test]
    fn collapses_consecutive_duplicates() {
        let mut s = Store::default();
        add(&mut s, "git status");
        s.add("git status", 1, "D:/", "bash", 9);
        add(&mut s, "ls");
        add(&mut s, "git status");
        let cmds: Vec<_> = s.entries.iter().map(|e| e.command.as_str()).collect();
        assert_eq!(cmds, ["git status", "ls", "git status"]);
        // 合并时刷新的是结果和时间
        assert_eq!((s.entries[0].exit_code, s.entries[0].at), (1, 9));
    }

    #[test]
    fn cap_never_evicts_favorites() {
        let mut s = Store::default();
        add(&mut s, "keep me");
        s.entries[0].favorite = true;
        for i in 0..MAX_ENTRIES + 10 {
            add(&mut s, &format!("cmd {i}"));
        }
        assert!(s.entries.iter().any(|e| e.command == "keep me"));
        assert_eq!(
            s.entries.iter().filter(|e| !e.favorite).count(),
            MAX_ENTRIES
        );
        // 删的是最旧的
        assert_eq!(
            s.entries.iter().find(|e| !e.favorite).unwrap().command,
            "cmd 10"
        );
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(new_nonce(), new_nonce());
        assert_eq!(new_nonce().len(), 32);
    }
}
