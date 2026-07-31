import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

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

// 单个树节点：文件夹单击展开、双击 cd；文件单击预览、右键「在终端显示」
function TreeNode({
  entry,
  depth,
  onOpenDir,
  onOpenFile,
  onShowInTerminal,
  showHidden,
  gitStatus,
  gitDeco,
  parentIgnored,
}: {
  entry: FileEntry;
  depth: number;
  onOpenDir: (path: string) => void;
  onOpenFile: (path: string, name: string) => void;
  onShowInTerminal: (path: string) => void;
  showHidden: boolean;
  gitStatus: GitStatus | null;
  gitDeco: boolean;
  parentIgnored: boolean;
}) {
  const t = useT();
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<FileEntry[] | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const onClick = async () => {
    if (!entry.is_dir) {
      onOpenFile(entry.path, entry.name);
      return;
    }
    if (!expanded && children === null) {
      try {
        setChildren(await invoke<FileEntry[]>("list_dir", { path: entry.path }));
      } catch {
        setChildren([]);
      }
    }
    setExpanded((e) => !e);
  };

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menu]);

  const visibleChildren = children?.filter(
    (c) => showHidden || !c.name.startsWith("."),
  );

  // git 装饰：文件取自身状态；文件夹看内部有没有改动（汇总一个点）
  const code = gitDeco && gitStatus ? gitStatus.files[entry.path] : undefined;
  const ignored = code === "!" || parentIgnored;
  const folderDirty =
    gitDeco && entry.is_dir && gitStatus
      ? Object.keys(gitStatus.files).some(
          (p) => gitStatus.files[p] !== "!" && p.startsWith(entry.path + "\\"),
        )
      : false;
  const nameCls =
    "tree-name" + (code && code !== "!" ? " git-" + statusClass(code) : "");

  return (
    <div>
      <div
        className={"tree-item" + (ignored ? " git-ignored" : "")}
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={onClick}
        onDoubleClick={() => entry.is_dir && onOpenDir(entry.path)}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          setMenu({ x: e.clientX, y: e.clientY });
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
      {menu && (
        <div
          className="ctx-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          {!entry.is_dir && (
            <div
              className="ctx-item"
              onClick={() => {
                onOpenFile(entry.path, entry.name);
                setMenu(null);
              }}
            >
              <span>{t("ctx.preview")}</span>
            </div>
          )}
          <div
            className="ctx-item"
            onClick={() => {
              onShowInTerminal(entry.path);
              setMenu(null);
            }}
          >
            <span>{t("ctx.showInTerminal")}</span>
          </div>
        </div>
      )}
      {expanded &&
        visibleChildren?.map((c) => (
          <TreeNode
            key={c.path}
            entry={c}
            depth={depth + 1}
            onOpenDir={onOpenDir}
            onOpenFile={onOpenFile}
            onShowInTerminal={onShowInTerminal}
            showHidden={showHidden}
            gitStatus={gitStatus}
            gitDeco={gitDeco}
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
  const [root, setRoot] = useState<FileEntry[]>([]);
  // 快速切目录/手动刷新会并发发出多个 list_dir，用请求序号只认最后一个发出的那个，
  // 避免慢的旧请求后返回，把新目录的结果覆盖掉
  const requestId = useRef(0);

  const load = (path: string) => {
    if (!path) return;
    const id = ++requestId.current;
    invoke<FileEntry[]>("list_dir", { path })
      .then((entries) => {
        if (id === requestId.current) setRoot(entries);
      })
      .catch(() => {
        if (id === requestId.current) setRoot([]);
      });
  };

  useEffect(() => {
    load(rootPath);
  }, [rootPath]);

  const rootName = rootPath.split(/[\\/]/).filter(Boolean).pop() ?? rootPath;
  const visibleRoot = root.filter((e) => showHidden || !e.name.startsWith("."));

  return (
    <>
      <div className="sidebar-head">
        <span title={rootPath}>{rootName.toUpperCase()}</span>
        <span
          className="sidebar-actions"
          title={t("sidebar.refresh")}
          onClick={() => load(rootPath)}
        >
          ⟳
        </span>
      </div>
      <div className="file-tree">
        {visibleRoot.map((e) => (
          <TreeNode
            key={rootPath + "|" + e.path}
            entry={e}
            depth={0}
            onOpenDir={onOpenDir}
            onOpenFile={onOpenFile}
            onShowInTerminal={onShowInTerminal}
            showHidden={showHidden}
            gitStatus={gitStatus}
            gitDeco={gitDeco}
            parentIgnored={false}
          />
        ))}
      </div>
    </>
  );
}
