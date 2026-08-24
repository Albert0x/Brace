import { useEffect, useState } from "react";
import { useT } from "../i18n";
import {
  emptySshSession,
  saveSshSessions,
  sshLabel,
  type SshSession,
} from "../hooks/useSshSessions";

// SSH 会话编辑面板。
//
// 这里管的只是「怎么拼那条 ssh 命令」——主机、端口、用户、密钥路径。
// **没有密码字段，而且不会有**：密码交互由 ssh 自己在 PTY 里完成，
// Brace 不接触 SSH 凭据，也就没有保管它的责任。
//
// 端口、用户、密钥留空是有意义的：留空就不往命令行里加对应参数，
// 让 ~/.ssh/config 说话。用户在那儿配好的 Host 别名和 ProxyJump 不该被我们覆盖。
export default function SshPanel({
  sessions: incoming,
  clientPath,
  onChanged,
}: {
  sessions: SshSession[];
  clientPath: string | null;
  onChanged: () => void;
}) {
  const t = useT();
  const [sessions, setSessions] = useState<SshSession[]>(incoming);
  const [selectedId, setSelectedId] = useState<string>(incoming[0]?.id ?? "");
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);

  // 面板重新打开时同步外面的最新数据，但正在编辑的内容不能被冲掉
  useEffect(() => {
    if (!dirty) {
      setSessions(incoming);
      setSelectedId((cur) => (incoming.some((s) => s.id === cur) ? cur : incoming[0]?.id ?? ""));
    }
  }, [incoming, dirty]);

  const current = sessions.find((s) => s.id === selectedId);

  const patch = (id: string, field: keyof SshSession, value: string | number) => {
    setSessions((list) =>
      list.map((s) => (s.id === id ? { ...s, [field]: value } : s)),
    );
    setDirty(true);
    setSaved(false);
    setError("");
  };

  const add = () => {
    const s = emptySshSession();
    setSessions((list) => [...list, s]);
    setSelectedId(s.id);
    setDirty(true);
    setSaved(false);
  };

  const remove = (id: string) => {
    setSessions((list) => {
      const next = list.filter((s) => s.id !== id);
      setSelectedId((cur) => (cur === id ? next[0]?.id ?? "" : cur));
      return next;
    });
    setDirty(true);
    setSaved(false);
  };

  const save = () => {
    if (!dirty || busy) return;
    // 主机地址是唯一不能缺的字段——没有它拼不出命令。在这儿拦下来，
    // 而不是等用户点了新建标签才看见后端报错
    const blank = sessions.find((s) => !s.host.trim());
    if (blank) {
      setError(t("ssh.needHost", { name: sshLabel(blank) || t("ssh.untitled") }));
      return;
    }
    setBusy(true);
    setError("");
    saveSshSessions(sessions.map((s) => ({ ...s, host: s.host.trim() })))
      .then(() => {
        setDirty(false);
        setSaved(true);
        onChanged();
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };

  // 系统里没有 ssh 客户端时，填什么都起不来。直接说清楚，别让人白填一张表
  if (!clientPath) {
    return (
      <div className="settings-section">
        <p className="settings-error">{t("ssh.noClient")}</p>
      </div>
    );
  }

  return (
    <div className="settings-section">
      <p className="settings-hint">{t("ssh.sub")}</p>

      <div className="settings-section-title">{t("ssh.list")}</div>
      <div className="prof-list">
        {sessions.map((s) => (
          <div
            key={s.id}
            className={"prof-list-item" + (s.id === selectedId ? " selected" : "")}
            onClick={() => setSelectedId(s.id)}
          >
            <span>{sshLabel(s) || t("ssh.untitled")}</span>
            <button
              className="prof-del"
              title={t("ssh.remove")}
              onClick={(e) => {
                e.stopPropagation();
                remove(s.id);
              }}
            >
              ×
            </button>
          </div>
        ))}
        <button className="bg-btn" onClick={add}>
          {t("ssh.new")}
        </button>
      </div>

      {current && (
        <>
          <div className="settings-section-title">{t("ssh.detail")}</div>
          <div className="ssh-form">
            <label>
              <span>{t("ssh.name")}</span>
              <input
                value={current.name}
                spellCheck={false}
                placeholder={t("ssh.namePlaceholder")}
                onChange={(e) => patch(current.id, "name", e.target.value)}
              />
            </label>
            <label>
              <span>{t("ssh.host")}</span>
              <input
                value={current.host}
                spellCheck={false}
                placeholder="example.com"
                onChange={(e) => patch(current.id, "host", e.target.value)}
              />
            </label>
            <label>
              <span>{t("ssh.user")}</span>
              <input
                value={current.user}
                spellCheck={false}
                placeholder={t("ssh.inheritConfig")}
                onChange={(e) => patch(current.id, "user", e.target.value)}
              />
            </label>
            <label>
              <span>{t("ssh.port")}</span>
              <input
                type="number"
                min={0}
                max={65535}
                value={current.port || ""}
                placeholder="22"
                onChange={(e) =>
                  patch(current.id, "port", Number(e.target.value) || 0)
                }
              />
            </label>
            <label>
              <span>{t("ssh.keyPath")}</span>
              <input
                value={current.keyPath}
                spellCheck={false}
                placeholder={t("ssh.inheritConfig")}
                onChange={(e) => patch(current.id, "keyPath", e.target.value)}
              />
            </label>
            <label>
              <span>{t("ssh.note")}</span>
              <input
                value={current.note}
                spellCheck={false}
                onChange={(e) => patch(current.id, "note", e.target.value)}
              />
            </label>
          </div>
          <p className="settings-hint">{t("ssh.blankMeansConfig")}</p>
          <p className="settings-hint">{t("ssh.noPassword")}</p>
        </>
      )}

      {error && <p className="settings-error">{error}</p>}

      <div className="bg-controls">
        <button className="bg-btn" disabled={!dirty || busy} onClick={save}>
          {busy ? t("ssh.saving") : t("ssh.save")}
        </button>
        {saved && !dirty && <span className="settings-hint">{t("ssh.saved")}</span>}
      </div>
    </div>
  );
}
