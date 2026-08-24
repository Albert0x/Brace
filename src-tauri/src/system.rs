use std::path::PathBuf;

use tauri::{AppHandle, Manager};

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
pub(crate) fn save_bg_image(app: AppHandle, data_url: String) -> Result<(), String> {
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
pub(crate) fn load_bg_image(app: AppHandle) -> Option<String> {
    std::fs::read_to_string(bg_path(&app).ok()?).ok()
}

// Win11 判断（build >= 22000）。Win10 的 acrylic 亚克力有边缘黑边 + 拖动卡顿，需区分。
#[cfg(target_os = "windows")]
pub(crate) fn is_win11() -> bool {
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
pub(crate) fn os_version() -> String {
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
pub(crate) fn system_proxy() -> Option<String> {
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
