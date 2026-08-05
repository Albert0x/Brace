import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  hidden: boolean; // Windows 隐藏属性，不是"名字以点开头"
}

/**
 * 文件树的状态：哪些目录展开着、各自的内容是什么。
 *
 * 这些原来分散在每个 TreeNode 自己的 useState 里，导致上层根本说不出
 * 「当前展开了哪些目录」——文件系统监听因此只能盯根目录一层，刷新也只能靠
 * 给每个节点发一个 token 让它们各自重新拉。收拢到这里之后，监听范围、
 * 精确刷新、以及后续任何需要全局视角的功能都有了着落。
 */
export function useFileTree(rootPath: string) {
  // 目录路径 → 它的直接子项。只有加载过的目录才在表里
  const [entries, setEntries] = useState<Record<string, FileEntry[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  // 快速切目录会并发发出多个 list_dir，用序号只认最后一次发出的那批，
  // 避免慢的旧请求后返回、把新目录的结果覆盖掉
  const generation = useRef(0);

  const readDir = useCallback(async (path: string, gen: number) => {
    try {
      const list = await invoke<FileEntry[]>("list_dir", { path });
      if (gen !== generation.current) return;
      setEntries((prev) => ({ ...prev, [path]: list }));
    } catch {
      if (gen !== generation.current) return;
      // 目录没了或没权限：记成空，树上表现为一个展不开的空目录，
      // 而不是一直停在上一次的内容上骗人
      setEntries((prev) => ({ ...prev, [path]: [] }));
    }
  }, []);

  // 换根目录：清空一切重来，旧目录的展开状态对新根没有意义
  useEffect(() => {
    generation.current++;
    setEntries({});
    setExpanded(new Set());
    if (rootPath) readDir(rootPath, generation.current);
  }, [rootPath, readDir]);

  // updater 必须是纯函数——StrictMode 下 React 会故意调用两遍来暴露副作用，
  // 把 readDir 写在里面就会发两次请求。所以先从 ref 判断方向，副作用留在外面
  const expandedRef = useRef(expanded);
  expandedRef.current = expanded;

  const toggle = useCallback(
    (path: string) => {
      const willExpand = !expandedRef.current.has(path);
      setExpanded((prev) => {
        const next = new Set(prev);
        if (willExpand) next.add(path);
        else next.delete(path);
        return next;
      });
      // 展开时拉一次，拿到的内容之后一直复用
      if (willExpand) readDir(path, generation.current);
    },
    [readDir],
  );

  // 重新加载根目录和所有展开着的目录。没展开的不动——等下次点开自然会加载，
  // 没必要为看不见的内容发请求
  const refresh = useCallback(() => {
    const gen = generation.current;
    if (rootPath) readDir(rootPath, gen);
    expanded.forEach((dir) => readDir(dir, gen));
  }, [rootPath, expanded, readDir]);

  // 后端监听到文件变化就重新拉。refresh 随 expanded 变化，用 ref 拿最新的那个，
  // 免得每展开一个目录就重订一次事件
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    const un = listen("fs-change", () => refreshRef.current());
    return () => {
      un.then((f) => f()).catch(() => {});
    };
  }, []);

  // 把要盯的目录报给后端：根目录 + 所有展开的。根目录放第一个，
  // 后端截断时先保住它
  useEffect(() => {
    const dirs = rootPath ? [rootPath, ...expanded] : [];
    invoke("watch_dirs", { paths: dirs }).catch(() => {});
  }, [rootPath, expanded]);

  // 某个目录内容变了（新建/重命名/删除之后）立刻重拉，不等文件系统事件那 500ms 防抖
  const reloadDir = useCallback(
    (dir: string) => readDir(dir, generation.current),
    [readDir],
  );

  // 删掉一个目录后，它和它的子孙不该继续留在展开集合里当幽灵
  const collapseSubtree = useCallback((path: string) => {
    setExpanded((prev) => {
      const next = new Set(
        [...prev].filter((p) => p !== path && !p.startsWith(path + "\\")),
      );
      return next.size === prev.size ? prev : next;
    });
  }, []);

  return {
    entries,
    expanded,
    toggle,
    refresh,
    reloadDir,
    collapseSubtree,
  };
}
