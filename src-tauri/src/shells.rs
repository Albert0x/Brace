use serde::Serialize;

// ---------- Shell 检测 ----------

#[derive(Serialize)]
pub(crate) struct ShellInfo {
    id: String,
    name: String,
    path: String,
    shell_type: String, // powershell | cmd | bash | zsh | sh
}

// 在 PATH 中查找可执行文件
pub(crate) fn which(exe: &str) -> Option<String> {
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
pub(crate) fn detect_shells() -> Vec<ShellInfo> {
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
