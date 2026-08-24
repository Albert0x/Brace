use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::shells::which;

// SSH 会话是「拼一条 ssh 命令」而已，不是一套自己的 SSH 实现。
//
// Windows 10 之后系统自带 OpenSSH 客户端，把它 spawn 进现有的 PTY，密钥、
// known_hosts、~/.ssh/config 全都沿用系统那一套。走 russh 之类的原生库意味着
// 自己实现密钥协商、主机指纹校验、keepalive、断线重连——那是几周的活，
// 而且还得自己保管凭据。
//
// **这里绝不存密码。** 密码交互在 PTY 里天然可用（ssh 自己问，用户自己答），
// 免密就用密钥。因此 Brace 全程不接触 SSH 凭据，也就没有保管它的责任。

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SshSession {
    pub(crate) id: String,
    // 显示名。为空时界面回退成 user@host
    pub(crate) name: String,
    pub(crate) host: String,
    // 0 表示不指定，交给 ssh 按 ~/.ssh/config 或默认 22 决定
    #[serde(default)]
    pub(crate) port: u16,
    // 为空则不拼 user@，让 ssh 用 config 里的 User 或当前账号
    #[serde(default)]
    pub(crate) user: String,
    // 私钥路径，为空则不传 -i，让 ssh 自己按 config 和默认位置找
    #[serde(default)]
    pub(crate) key_path: String,
    #[serde(default)]
    pub(crate) note: String,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SshStore {
    #[serde(default)]
    pub(crate) sessions: Vec<SshSession>,
}

fn ssh_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("拿不到配置目录：{}", e))?;
    Ok(dir.join("ssh.json"))
}

// 文件不存在或内容坏了都当成「还没有会话」，不要在启动路径上抛错
fn read_store(app: &AppHandle) -> SshStore {
    ssh_path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub(crate) fn load_ssh_sessions(app: AppHandle) -> SshStore {
    read_store(&app)
}

#[tauri::command]
pub(crate) fn save_ssh_sessions(app: AppHandle, store: SshStore) -> Result<(), String> {
    // 主机名是这条记录唯一不能缺的东西——没有它拼不出命令
    if let Some(bad) = store.sessions.iter().find(|s| s.host.trim().is_empty()) {
        return Err(format!("会话「{}」没有填主机地址", display_name(bad)));
    }
    let path = ssh_path(&app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 临时文件 + rename，写一半崩了也不会留下半个损坏的配置
    let text = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

// 界面上怎么称呼这个会话
pub(crate) fn display_name(s: &SshSession) -> String {
    if !s.name.trim().is_empty() {
        return s.name.trim().to_string();
    }
    if s.user.trim().is_empty() {
        s.host.trim().to_string()
    } else {
        format!("{}@{}", s.user.trim(), s.host.trim())
    }
}

// 拼 ssh 的参数表。
//
// 原则：只拼用户明确填了的东西。端口留空、密钥留空、用户名留空，就一个字都不加——
// 那些全都交给 ~/.ssh/config。用户在那儿配好的 Host 别名、ProxyJump、IdentityFile
// 我们一个都不该覆盖，否则「在 Brace 里连不上但在终端里能连」就会变成常见投诉。
pub(crate) fn ssh_args(s: &SshSession) -> Vec<String> {
    let mut args = Vec::new();
    // 22 和 0 都视为「没指定」
    if s.port != 0 && s.port != 22 {
        args.push("-p".into());
        args.push(s.port.to_string());
    }
    let key = s.key_path.trim();
    if !key.is_empty() {
        args.push("-i".into());
        args.push(key.to_string());
    }
    let host = s.host.trim();
    let user = s.user.trim();
    args.push(if user.is_empty() {
        host.to_string()
    } else {
        format!("{}@{}", user, host)
    });
    args
}

// 系统自带的 ssh 客户端。Windows 10 起在 System32\OpenSSH 下，同时也在 PATH 里
#[tauri::command]
pub(crate) fn ssh_client_path() -> Option<String> {
    which("ssh")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SshLaunch {
    path: String,
    args: Vec<String>,
}

// 前端开 SSH 标签前问这里：客户端在哪、参数怎么拼。
//
// 拼装逻辑留在 Rust 侧是为了只有一份实现——它有一整套测试盯着「留空就不传」
// 这个约定。让前端照着字段自己拼，等于把同样的判断再写一遍 TypeScript，
// 两份实现迟早会分叉。
#[tauri::command]
pub(crate) fn ssh_launch(app: AppHandle, session_id: String) -> Result<SshLaunch, String> {
    let session = read_store(&app)
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)
        .ok_or("找不到这个 SSH 会话")?;
    let path = which("ssh").ok_or("找不到 ssh 客户端（需要 Windows 自带的 OpenSSH）")?;
    Ok(SshLaunch {
        path,
        args: ssh_args(&session),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SshSession {
        SshSession {
            id: "1".into(),
            host: "example.com".into(),
            ..Default::default()
        }
    }

    #[test]
    fn omits_everything_the_user_did_not_fill_in() {
        // 这条是整个模块的核心约定：留空就不传，交给 ~/.ssh/config。
        // 一旦这里开始「贴心地」补默认值，用户 config 里的设置就会被悄悄覆盖
        assert_eq!(ssh_args(&session()), vec!["example.com"]);
    }

    #[test]
    fn treats_port_22_as_unspecified() {
        // 显式写 22 和不写是一个意思，没必要往命令行里塞 -p 22
        let s = SshSession {
            port: 22,
            ..session()
        };
        assert_eq!(ssh_args(&s), vec!["example.com"]);
    }

    #[test]
    fn passes_a_non_default_port() {
        let s = SshSession {
            port: 2222,
            ..session()
        };
        assert_eq!(ssh_args(&s), vec!["-p", "2222", "example.com"]);
    }

    #[test]
    fn prefixes_the_user_when_given() {
        let s = SshSession {
            user: "root".into(),
            ..session()
        };
        assert_eq!(ssh_args(&s), vec!["root@example.com"]);
    }

    #[test]
    fn passes_an_identity_file_when_given() {
        let s = SshSession {
            key_path: "C:\\keys\\id_ed25519".into(),
            ..session()
        };
        assert_eq!(
            ssh_args(&s),
            vec!["-i", "C:\\keys\\id_ed25519", "example.com"]
        );
    }

    #[test]
    fn combines_all_of_them_in_ssh_argument_order() {
        let s = SshSession {
            port: 2222,
            user: "root".into(),
            key_path: "k".into(),
            ..session()
        };
        assert_eq!(
            ssh_args(&s),
            vec!["-p", "2222", "-i", "k", "root@example.com"]
        );
    }

    #[test]
    fn trims_whitespace_out_of_fields() {
        // 从别处复制粘贴过来的值经常带空格，带着空格拼进命令行会直接连不上
        let s = SshSession {
            host: "  example.com  ".into(),
            user: " root ".into(),
            key_path: "  k  ".into(),
            ..session()
        };
        assert_eq!(ssh_args(&s), vec!["-i", "k", "root@example.com"]);
    }

    #[test]
    fn falls_back_to_user_at_host_when_unnamed() {
        assert_eq!(display_name(&session()), "example.com");
        let s = SshSession {
            user: "root".into(),
            ..session()
        };
        assert_eq!(display_name(&s), "root@example.com");
    }

    #[test]
    fn prefers_an_explicit_name() {
        let s = SshSession {
            name: "prod-db".into(),
            user: "root".into(),
            ..session()
        };
        assert_eq!(display_name(&s), "prod-db");
    }

    #[test]
    fn stored_form_survives_a_round_trip() {
        let s = SshSession {
            id: "abc".into(),
            name: "prod".into(),
            host: "10.0.0.1".into(),
            port: 2222,
            user: "root".into(),
            key_path: "k".into(),
            note: "生产库".into(),
        };
        let store = SshStore {
            sessions: vec![s.clone()],
        };
        let text = serde_json::to_string(&store).expect("序列化");
        // 落盘用 camelCase：前端直接消费，不需要额外转换
        assert!(text.contains("\"keyPath\""));
        assert!(!text.contains("password"), "这里永远不该出现密码字段");

        let back: SshStore = serde_json::from_str(&text).expect("反序列化");
        assert_eq!(back.sessions.len(), 1);
        assert_eq!(back.sessions[0].host, s.host);
        assert_eq!(back.sessions[0].port, s.port);
        assert_eq!(back.sessions[0].key_path, s.key_path);
    }

    #[test]
    fn reads_a_minimal_record_written_by_hand() {
        // 用户手写配置文件时不会把字段填全，缺的都得有默认值
        let back: SshStore =
            serde_json::from_str(r#"{"sessions":[{"id":"1","name":"","host":"h"}]}"#)
                .expect("反序列化");
        assert_eq!(back.sessions[0].port, 0);
        assert!(back.sessions[0].user.is_empty());
        assert_eq!(ssh_args(&back.sessions[0]), vec!["h"]);
    }

    #[test]
    fn tolerates_a_completely_empty_file() {
        let back: SshStore = serde_json::from_str("{}").expect("反序列化");
        assert!(back.sessions.is_empty());
    }
}
