import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useT } from "../i18n";
import { useFileTree, type FileEntry } from "../hooks/useFileTree";
import FileEntryDialog, { type EntryAction } from "./FileEntryDialog";

export interface GitStatus {
  isRepo: boolean;
  branch: string;
  changedCount: number;
  files: Record<string, string>; // 绝对路径 → M/A/?/D/R/!
}

// git 状态码 → CSS 类名后缀
function statusClass(code: string): string {
  switch (code) {
    case "A":
      return "added";
    case "?":
      return "untracked";
    case "D":
      return "deleted";
    case "R":
      return "renamed";
    default:
      return "modified";
  }
}

const parentOf = (path: string) => path.slice(0, path.lastIndexOf("\\"));

interface NodeHandlers {
  onOpenDir: (path: string) => void;
  onOpenFile: (path: string, name: string) => void;
  onShowInTerminal: (path: string) => void;
  onContextMenu: (entry: FileEntry, x: number, y: number) => void;
}

// 单个树节点。展开状态和子项内容都由上层的 useFileTree 持有，这里只负责画
function TreeNode({
  entry,
  depth,
  tree,
  handlers,
  showHidden,
  gitStatus,
  gitDeco,
  dirtyDirs,
  parentIgnored,
}: {
  entry: FileEntry;
  depth: number;
  tree: ReturnType<typeof useFileTree>;
  handlers: NodeHandlers;
  showHidden: boolean;
  gitStatus: GitStatus | null;
  gitDeco: boolean;
  dirtyDirs: Set<string>;
  parentIgnored: boolean;
}) {
  const t = useT();
  const expanded = entry.is_dir && tree.expanded.has(entry.path);
  const children = tree.entries[entry.path];

  // git 装饰：文件取自身状态；文件夹看内部有没有改动（汇总一个点）
  const code = gitDeco && gitStatus ? gitStatus.files[entry.path] : undefined;
  const ignored = code === "!" || parentIgnored;
  // 脏目录集合在上层一次性算好，这里 O(1) 查表
  const folderDirty = entry.is_dir && dirtyDirs.has(entry.path);
  const nameCls =
    "tree-name" + (code && code !== "!" ? " git-" + statusClass(code) : "");

  return (
    <div>
      <div
        className={"tree-item" + (ignored ? " git-ignored" : "")}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={() =>
          entry.is_dir
            ? tree.toggle(entry.path)
            : handlers.onOpenFile(entry.path, entry.name)
        }
        onDoubleClick={() => entry.is_dir && handlers.onOpenDir(entry.path)}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          handlers.onContextMenu(entry, e.clientX, e.clientY);
        }}
        title={entry.is_dir ? t("tree.enterDir", { path: entry.path }) : entry.path}
      >
        <span className="tree-arrow">
          {entry.is_dir ? (expanded ? "▾" : "▸") : ""}
        </span>
        <span className="tree-icon">{entry.is_dir ? "📁" : "📄"}</span>
        <span className={nameCls}>{entry.name}</span>
        {code && code !== "!" && (
          <span className={"git-badge git-" + statusClass(code)}>{code}</span>
        )}
        {folderDirty && !code && <span className="git-dot" />}
      </div>
      {expanded &&
        children
          ?.filter((c) => showHidden || !c.hidden)
          .map((c) => (
            <TreeNode
              key={c.path}
              entry={c}
              depth={depth + 1}
              tree={tree}
              handlers={handlers}
              showHidden={showHidden}
              gitStatus={gitStatus}
              gitDeco={gitDeco}
              dirtyDirs={dirtyDirs}
              parentIgnored={ignored}
            />
          ))}
    </div>
  );
}

// 文件树：根目录受控（跟随活动终端 cwd），双击文件夹反向驱动终端 cd
export default function FileTree({
  rootPath,
  onOpenDir,
  onOpenFile,
  onShowInTerminal,
  showHidden,
  gitStatus,
  gitDeco,
}: {
  rootPath: string;
  onOpenDir: (path: string) => void;
  onOpenFile: (path: string, name: string) => void;
  onShowInTerminal: (path: string) => void;
  showHidden: boolean;
  gitStatus: GitStatus | null;
  gitDeco: boolean;
}) {
  const t = useT();
  const tree = useFileTree(rootPath);

  const [menu, setMenu] = useState<{
    entry: FileEntry | null; // null = 在空白处右键，操作对象是根目录
    x: number;
    y: number;
  } | null>(null);
  const [dialog, setDialog] = useState<{
    action: EntryAction;
    entry: FileEntry | null;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menu]);

  // 把每个改动文件的各级父目录预先标脏，树上查表就行。
  // 自底向上走，碰到已标过的祖先就停——它上面的必然也已经标过了
  const dirtyDirs = useMemo(() => {
    const dirs = new Set<string>();
    if (!gitDeco || !gitStatus) return dirs;
    for (const [file, code] of Object.entries(gitStatus.files)) {
      if (code === "!") continue;
      let cut = file.lastIndexOf("\\");
      while (cut > 0) {
        const dir = file.slice(0, cut);
        if (dirs.has(dir)) break;
        dirs.add(dir);
        cut = dir.lastIndexOf("\\");
      }
    }
    return dirs;
  }, [gitStatus, gitDeco]);

  const openDialog = (action: EntryAction, entry: FileEntry | null) => {
    setError("");
    setDialog({ action, entry });
    setMenu(null);
  };

  // 新建落在哪个目录：对着文件夹右键就是它自己，对着文件或空白就是所在目录
  const targetDir = (entry: FileEntry | null) =>
    !entry ? rootPath : entry.is_dir ? entry.path : parentOf(entry.path);

  const runDialog = async (name: string) => {
    if (!dialog) return;
    const { action, entry } = dialog;
    setBusy(true);
    setError("");
    try {
      if (action === "delete") {
        if (!entry) return;
        await invoke("delete_entry", { path: entry.path });
        // 删的是目录的话，它和子孙不能继续赖在展开集合里
        if (entry.is_dir) tree.collapseSubtree(entry.path);
        tree.reloadDir(parentOf(entry.path));
      } else if (action === "rename") {
        if (!entry) return;
        await invoke("rename_entry", { path: entry.path, name });
        if (entry.is_dir) tree.collapseSubtree(entry.path);
        tree.reloadDir(parentOf(entry.path));
      } else {
        const dir = targetDir(entry);
        await invoke("create_entry", {
          parent: dir,
          name,
          isDir: action === "newFolder",
        });
        tree.reloadDir(dir);
      }
      setDialog(null);
    } catch (e) {
      // 失败就把对话框留着显示原因，用户可以改个名字再试，不用从右键重来
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const rootName = rootPath.split(/[\\/]/).filter(Boolean).pop() ?? rootPath;
  const rootEntries = tree.entries[rootPath] ?? [];
  const visibleRoot = rootEntries.filter((e) => showHidden || !e.hidden);

  const handlers: NodeHandlers = {
    onOpenDir,
    onOpenFile,
    onShowInTerminal,
    onContextMenu: (entry, x, y) => setMenu({ entry, x, y }),
  };

  const menuEntry = menu?.entry ?? null;

  return (
    <>
      <div className="sidebar-head">
        <span title={rootPath}>{rootName.toUpperCase()}</span>
        <span
          className="sidebar-actions"
          title={t("tree.newFile")}
          onClick={() => openDialog("newFile", null)}
        >
          ＋
        </span>
        <span
          className="sidebar-actions"
          title={t("sidebar.refresh")}
          onClick={tree.refresh}
        >
          ⟳
        </span>
      </div>

      <div
        className="file-tree"
        onContextMenu={(e) => {
          // 空白处右键：操作对象是根目录，方便直接在根下新建
          e.preventDefault();
          setMenu({ entry: null, x: e.clientX, y: e.clientY });
        }}
      >
        {visibleRoot.map((e) => (
          <TreeNode
            key={e.path}
            entry={e}
            depth={0}
            tree={tree}
            handlers={handlers}
            showHidden={showHidden}
            gitStatus={gitStatus}
            gitDeco={gitDeco}
            dirtyDirs={dirtyDirs}
            parentIgnored={false}
          />
        ))}
      </div>

      {menu && (
        <div
          className="ctx-menu"
          // 靠近屏幕下缘时向上展开，免得菜单被切掉一半
          style={
            menu.y > window.innerHeight / 2
              ? { left: menu.x, bottom: window.innerHeight - menu.y }
              : { left: menu.x, top: menu.y }
          }
          onClick={(e) => e.stopPropagation()}
        >
          {menuEntry && !menuEntry.is_dir && (
            <div
              className="ctx-item"
              onClick={() => {
                onOpenFile(menuEntry.path, menuEntry.name);
                setMenu(null);
              }}
            >
              <span>{t("ctx.preview")}</span>
            </div>
          )}
          {menuEntry && (
            <div
              className="ctx-item"
              onClick={() => {
                if (menuEntry.is_dir) onOpenDir(menuEntry.path);
                else onShowInTerminal(menuEntry.path);
                setMenu(null);
              }}
            >
              <span>
                {menuEntry.is_dir ? t("ctx.cdHere") : t("ctx.showInTerminal")}
              </span>
            </div>
          )}
          {menuEntry && <div className="ctx-sep" />}

          <div className="ctx-item" onClick={() => openDialog("newFile", menuEntry)}>
            <span>{t("tree.newFile")}</span>
          </div>
          <div
            className="ctx-item"
            onClick={() => openDialog("newFolder", menuEntry)}
          >
            <span>{t("tree.newFolder")}</span>
          </div>

          {menuEntry && (
            <>
              <div className="ctx-sep" />
              <div
                className="ctx-item"
                onClick={() => openDialog("rename", menuEntry)}
              >
                <span>{t("tree.rename")}</span>
              </div>
              <div
                className="ctx-item danger"
                onClick={() => openDialog("delete", menuEntry)}
              >
                <span>{t("tree.delete")}</span>
              </div>
            </>
          )}

          <div className="ctx-sep" />
          <div
            className="ctx-item"
            onClick={() => {
              navigator.clipboard
                .writeText(menuEntry?.path ?? rootPath)
                .catch(() => {});
              setMenu(null);
            }}
          >
            <span>{t("tree.copyPath")}</span>
          </div>
          <div
            className="ctx-item"
            onClick={() => {
              revealItemInDir(menuEntry?.path ?? rootPath).catch(console.error);
              setMenu(null);
            }}
          >
            <span>{t("tree.revealInExplorer")}</span>
          </div>
        </div>
      )}

      {dialog && (
        <FileEntryDialog
          action={dialog.action}
          target={
            dialog.action === "newFile" || dialog.action === "newFolder"
              ? targetDir(dialog.entry).split("\\").pop() || rootName
              : (dialog.entry?.name ?? "")
          }
          busy={busy}
          error={error}
          onCancel={() => setDialog(null)}
          onSubmit={runDialog}
        />
      )}
    </>
  );
}
