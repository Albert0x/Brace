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

// 从 git.exe 的位置反推 Git Bash。PATH 里第一个 git.exe 可能是这三个里的任意一个：
//   <root>\cmd\git.exe           开始菜单 / 资源管理器启动时通常是它
//   <root>\bin\git.exe
//   <root>\mingw64\bin\git.exe  从 Git Bash 里启动 Brace 时排在最前面
// 以前固定往上退两级，只认第一种；碰上第三种会退到 <root>\mingw64，找不到 bash，
// Git Bash 就从菜单里消失了——Git 不在 C:\Program Files 时连兜底都救不回来。
// 所以沿祖先目录往上找，最多三级：够覆盖上面三种，又不至于一路找到盘符根
#[cfg(windows)]
fn git_bash_from_git(git: &std::path::Path) -> Option<std::path::PathBuf> {
    git.parent()?
        .ancestors()
        .take(3)
        .map(|dir| dir.join("bin").join("bash.exe"))
        .find(|b| b.is_file())
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
        // Git Bash：先从 git.exe 反推安装根，不管 Git 装哪都能找到；找不到再退回标准路径
        let mut bash_path: Option<String> = which("git.exe")
            .and_then(|git| git_bash_from_git(std::path::Path::new(&git)))
            .map(|b| b.to_string_lossy().to_string());
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::Path;

    // 造一个假的 Git 安装目录：只放 bin\bash.exe，其余目录建出来就行。
    // 函数只按路径推，不会真去执行 git.exe
    fn fake_git_root(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("brace-gitbash-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["bin", "cmd", "mingw64/bin"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("bin").join("bash.exe"), b"").unwrap();
        root
    }

    #[test]
    fn finds_bash_from_every_git_location() {
        let root = fake_git_root("all");
        let want = root.join("bin").join("bash.exe");
        for git in ["cmd/git.exe", "bin/git.exe", "mingw64/bin/git.exe"] {
            assert_eq!(
                git_bash_from_git(&root.join(git)).as_deref(),
                Some(want.as_path()),
                "从 {} 没找到 bash",
                git
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_bash_means_none() {
        let root = fake_git_root("none");
        std::fs::remove_file(root.join("bin").join("bash.exe")).unwrap();
        assert_eq!(git_bash_from_git(&root.join("mingw64/bin/git.exe")), None);
        assert_eq!(git_bash_from_git(Path::new("git.exe")), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
