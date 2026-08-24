use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

// ---------- 文件系统 ----------

#[derive(Serialize)]
pub(crate) struct FileEntry {
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
pub(crate) fn list_dir(path: String) -> Result<Vec<FileEntry>, String> {
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
pub(crate) fn home_dir() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| "C:\\".to_string())
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
pub(crate) struct FsWatcher(Mutex<WatchState>);

// 监听目录数上限。展开 64 个目录已经远超正常使用，真到了这个量级也说明
// 再多盯几个也没意义，不如给个明确的天花板
const MAX_WATCHED_DIRS: usize = 64;

#[tauri::command]
pub(crate) fn watch_dirs(
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
pub(crate) fn create_entry(parent: String, name: String, is_dir: bool) -> Result<String, String> {
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
pub(crate) fn rename_entry(path: String, name: String) -> Result<String, String> {
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
pub(crate) fn delete_entry(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    trash::delete(&path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
