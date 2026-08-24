use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
pub(crate) struct UiStore {
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
pub(crate) struct InStore {
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
pub(crate) fn load_profiles(app: AppHandle) -> UiStore {
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
pub(crate) fn save_profiles(app: AppHandle, store: InStore) -> Result<(), String> {
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
pub(crate) fn active_env(app: &AppHandle) -> Vec<(String, String)> {
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
