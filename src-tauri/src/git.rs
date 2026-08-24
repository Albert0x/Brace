use serde::Serialize;
use std::collections::HashMap;

use crate::preview::decode_text;

// ---------- Git 装饰 ----------

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GitStatus {
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
pub(crate) fn git_status(cwd: String, include_ignored: bool) -> GitStatus {
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
pub(crate) struct CommitOutcome {
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
pub(crate) fn git_commit(
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
pub(crate) fn git_diff(cwd: String, path: String) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    // ----- 提交结果的三态（真实仓库） -----

    // 造一个临时的、没有 remote 的 git 仓库。进程 id 进目录名，避免并发跑测试时撞车
    #[cfg(windows)]
    fn temp_repo(tag: &str) -> Option<String> {
        let dir = std::env::temp_dir().join(format!("brace-test-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let cwd = dir.to_str()?.to_string();
        // git 不可用就放弃这条测试，而不是把它判成失败
        run_git_out(&cwd, &["init"], false).ok()?;
        run_git_out(&cwd, &["config", "user.name", "Brace Test"], false).ok()?;
        run_git_out(
            &cwd,
            &["config", "user.email", "test@example.invalid"],
            false,
        )
        .ok()?;
        Some(cwd)
    }

    // 这条盯的是最容易诱导用户做错事的那个路径：提交成功、推送失败。
    // 一旦它被报成「整体失败」，用户就会再点一次提交——然后撞上 nothing to commit,
    // 或者多出一个空提交。所以断言不止看返回值，还要回查提交数量。
    #[test]
    #[cfg(windows)]
    fn reports_commit_succeeded_even_when_push_fails() {
        let Some(cwd) = temp_repo("push-fail") else {
            eprintln!("跳过：git 不可用");
            return;
        };
        std::fs::write(format!("{cwd}/a.txt"), "hello").expect("写测试文件");

        // 仓库没有 remote，push 必然失败
        let outcome = git_commit(cwd.clone(), "test: first".into(), true, vec![], true)
            .expect("commit 本身必须成功");

        assert!(outcome.committed, "提交确实发生了");
        assert!(!outcome.pushed, "没有 remote，不可能推上去");
        assert!(
            outcome.push_error.is_some(),
            "推送失败必须带上原因，否则前端没法解释发生了什么"
        );

        let log = run_git_out(&cwd, &["log", "--oneline"], true).expect("读取提交历史");
        assert_eq!(
            log.lines().filter(|l| !l.trim().is_empty()).count(),
            1,
            "只应该有一个提交"
        );

        let _ = std::fs::remove_dir_all(std::path::Path::new(&cwd));
    }

    #[test]
    #[cfg(windows)]
    fn reports_plain_commit_when_push_not_requested() {
        let Some(cwd) = temp_repo("no-push") else {
            eprintln!("跳过：git 不可用");
            return;
        };
        std::fs::write(format!("{cwd}/a.txt"), "hello").expect("写测试文件");

        let outcome = git_commit(cwd.clone(), "test: first".into(), false, vec![], true)
            .expect("commit 必须成功");

        assert!(outcome.committed);
        assert!(!outcome.pushed);
        assert!(
            outcome.push_error.is_none(),
            "没要求推送就不该有推送错误——前端会据此显示成功而不是警告"
        );

        let _ = std::fs::remove_dir_all(std::path::Path::new(&cwd));
    }

    #[test]
    #[cfg(windows)]
    fn refuses_to_commit_without_message() {
        let Some(cwd) = temp_repo("empty-msg") else {
            eprintln!("跳过：git 不可用");
            return;
        };
        std::fs::write(format!("{cwd}/a.txt"), "hello").expect("写测试文件");
        assert!(git_commit(cwd.clone(), "   ".into(), false, vec![], true).is_err());
        let _ = std::fs::remove_dir_all(std::path::Path::new(&cwd));
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
}
