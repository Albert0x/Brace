use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::pty::PtyManager;
use crate::shells::which;

// ---------- Claude 用量统计 ----------
// 数据源：~/.claude/statusline-cache.json，由 Brace 的 statusLine 采集脚本写入
// （脚本接住 Claude Code 通过 statusLine stdin 喂的官方运行时数据）。
// 这里只负责：① 判断当前标签是否真在跑 claude；② 读缓存把官方 context/5h/7d 吐给前端。

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageStats {
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
pub(crate) fn usage_stats(manager: State<'_, PtyManager>, session_id: String) -> UsageStats {
    let mut stats = UsageStats::default();

    // 当前标签在跑什么 agent；都没跑就返回空，前端隐藏整条
    let shell_pid = manager.pid_of(&session_id);
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
pub(crate) struct StatuslineStatus {
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
pub(crate) fn statusline_status(app: AppHandle) -> StatuslineStatus {
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
pub(crate) fn configure_statusline(
    app: AppHandle,
    enable: bool,
    force: bool,
) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
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
