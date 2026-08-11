import { useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { GitStatus } from "../components/FileTree";
import { usePolling } from "./usePolling";

// 当前目录的 git 状态（分支 + 每个文件的状态），随目录变化 + 定时刷新。
// 20s 一轮：git status 带 --ignored 比不带贵几倍，而分支和改动列表没那么急。
// 要立刻刷新的场合（刚提交完）调 refresh。
//
// includeIgnored 跟着「Git 装饰」开关走：被忽略的文件只有文件树的装饰用得到，
// 装饰关着的时候还去枚举 node_modules 里的几万条路径纯属白烧
export function useGitStatus(cwd: string, includeIgnored: boolean) {
  const [gitStatus, setGitStatus] = useState<GitStatus | null>(null);

  const refresh = useCallback(() => {
    invoke<GitStatus>("git_status", { cwd, includeIgnored })
      .then((g) => setGitStatus(g.isRepo ? g : null))
      .catch(() => {});
  }, [cwd, includeIgnored]);

  usePolling(refresh, 20000, [cwd, includeIgnored]);

  return { gitStatus, refresh };
}
