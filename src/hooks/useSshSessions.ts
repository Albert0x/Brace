import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface SshSession {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  keyPath: string;
  note: string;
}

// SSH 会话列表。存的是「怎么拼那条 ssh 命令」，**不含密码**——
// 密码交互在 PTY 里由 ssh 自己完成，Brace 全程不接触 SSH 凭据
export function useSshSessions() {
  const [sessions, setSessions] = useState<SshSession[]>([]);
  // 系统里 ssh 客户端的路径。null 表示没找到，此时整个 SSH 入口都该收起来，
  // 而不是让用户填完一张表、点了新建标签才发现起不来
  const [clientPath, setClientPath] = useState<string | null>(null);

  const refresh = useCallback(() => {
    invoke<{ sessions: SshSession[] }>("load_ssh_sessions")
      .then((s) => setSessions(Array.isArray(s.sessions) ? s.sessions : []))
      .catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    invoke<string | null>("ssh_client_path")
      .then(setClientPath)
      .catch(() => setClientPath(null));
  }, [refresh]);

  return { sessions, clientPath, refresh };
}

export const saveSshSessions = (sessions: SshSession[]) =>
  invoke("save_ssh_sessions", { store: { sessions } });

export const emptySshSession = (): SshSession => ({
  id: crypto.randomUUID(),
  name: "",
  host: "",
  // 0 表示不指定，交给 ssh 按 ~/.ssh/config 或默认 22 决定
  port: 0,
  user: "",
  keyPath: "",
  note: "",
});

// 界面上怎么称呼一个会话。和 Rust 侧的 display_name 保持同一套规则
export const sshLabel = (s: SshSession) => {
  if (s.name.trim()) return s.name.trim();
  if (!s.user.trim()) return s.host.trim();
  return `${s.user.trim()}@${s.host.trim()}`;
};
