import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

// 单个树节点：单击展开/折叠，双击文件夹让终端 cd 过去
function TreeNode({
  entry,
  depth,
  onOpenDir,
  showHidden,
}: {
  entry: FileEntry;
  depth: number;
  onOpenDir: (path: string) => void;
  showHidden: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [children, setChildren] = useState<FileEntry[] | null>(null);

  const toggle = async () => {
    if (!entry.is_dir) return;
    if (!expanded && children === null) {
      try {
        setChildren(await invoke<FileEntry[]>("list_dir", { path: entry.path }));
      } catch {
        setChildren([]);
      }
    }
    setExpanded((e) => !e);
  };

  const visibleChildren = children?.filter(
    (c) => showHidden || !c.name.startsWith("."),
  );

  return (
    <div>
      <div
        className="tree-item"
        style={{ paddingLeft: 8 + depth * 14 }}
        onClick={toggle}
        onDoubleClick={() => entry.is_dir && onOpenDir(entry.path)}
        title={entry.is_dir ? `双击进入：${entry.path}` : entry.path}
      >
        <span className="tree-arrow">
          {entry.is_dir ? (expanded ? "▾" : "▸") : ""}
        </span>
        <span className="tree-icon">{entry.is_dir ? "📁" : "📄"}</span>
        <span className="tree-name">{entry.name}</span>
      </div>
      {expanded &&
        visibleChildren?.map((c) => (
          <TreeNode
            key={c.path}
            entry={c}
            depth={depth + 1}
            onOpenDir={onOpenDir}
            showHidden={showHidden}
          />
        ))}
    </div>
  );
}

// 文件树：根目录受控（跟随活动终端 cwd），双击文件夹反向驱动终端 cd
export default function FileTree({
  rootPath,
  onOpenDir,
  showHidden,
}: {
  rootPath: string;
  onOpenDir: (path: string) => void;
  showHidden: boolean;
}) {
  const [root, setRoot] = useState<FileEntry[]>([]);

  const load = (path: string) => {
    if (!path) return;
    invoke<FileEntry[]>("list_dir", { path })
      .then(setRoot)
      .catch(() => setRoot([]));
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
          title="刷新"
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
            showHidden={showHidden}
          />
        ))}
      </div>
    </>
  );
}
