import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useT } from "../i18n";
import {
  loadProfiles,
  saveProfiles,
  EMPTY_STORE,
  type Profile,
  type ProfileStore,
  type ProfileVar,
} from "../hooks/useProfiles";

// 变量名里带这些词的默认按密钥处理（落盘加密 + 界面隐藏）
const SECRET_HINT = /(TOKEN|KEY|SECRET|PASSWORD|PASSWD|CREDENTIAL)/i;

const V = (key: string, secret = SECRET_HINT.test(key)): ProfileVar => ({
  key,
  value: "",
  secret,
  hasValue: false,
});

// 预设模板：只是往表里填几个常见变量名，值还得用户自己填
const TEMPLATES: Record<string, ProfileVar[]> = {
  claude: [
    V("ANTHROPIC_BASE_URL"),
    V("ANTHROPIC_AUTH_TOKEN"),
    V("ANTHROPIC_MODEL"),
  ],
  codex: [V("OPENAI_BASE_URL"), V("OPENAI_API_KEY")],
  proxy: [
    V("HTTP_PROXY"),
    V("HTTPS_PROXY"),
    V("ALL_PROXY"),
    { ...V("NO_PROXY"), value: "localhost,127.0.0.1" },
  ],
};

// 环境变量配置组编辑面板（挂在设置面板的一个 tab 里）。
// 自己负责 load/save，改完通过 onChanged 通知外面刷新状态栏那颗徽标
export default function ProfilePanel({ onChanged }: { onChanged: () => void }) {
  const t = useT();
  const [store, setStore] = useState<ProfileStore>(EMPTY_STORE);
  const [selected, setSelected] = useState("");
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    loadProfiles().then((s) => {
      setStore(s);
      setSelected(s.activeId || s.profiles[0]?.id || "");
    });
  }, []);

  // 切到别的配置组时把"确认删除"状态收回来，否则手一快就删错组了
  useEffect(() => setConfirmDelete(false), [selected]);

  const current = store.profiles.find((p) => p.id === selected) ?? null;

  const patch = (fn: (p: Profile) => Profile) => {
    setStore((s) => ({
      ...s,
      profiles: s.profiles.map((p) => (p.id === selected ? fn(p) : p)),
    }));
    setDirty(true);
    setNotice("");
  };

  const setVars = (fn: (vars: ProfileVar[]) => ProfileVar[]) =>
    patch((p) => ({ ...p, vars: fn(p.vars) }));

  const addProfile = () => {
    const id = crypto.randomUUID();
    setStore((s) => ({
      ...s,
      profiles: [...s.profiles, { id, name: "", vars: [V("")] }],
    }));
    setSelected(id);
    setDirty(true);
    setNotice("");
  };

  // 删除做成两段式（点一次问、再点才删），不用 window.confirm——
  // 那是个原生阻塞弹窗，跟面板其它交互的观感也对不上
  const deleteProfile = () => {
    if (!current) return;
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    setConfirmDelete(false);
    const next = store.profiles.filter((p) => p.id !== selected);
    // 删掉的正好是当前生效的那个，顺手把 activeId 也摘掉，别留个指向空气的 id
    const nextStore: ProfileStore = {
      ...store,
      profiles: next,
      activeId: store.activeId === selected ? "" : store.activeId,
    };
    setStore(nextStore);
    setSelected(next[0]?.id ?? "");
    // 删除直接落盘，不留在 dirty 状态里等用户再点一次保存
    setBusy(true);
    saveProfiles(nextStore)
      .then(() => {
        setDirty(false);
        setNotice(t("profiles.saved"));
        onChanged();
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };

  // 模板：已存在的变量名不重复添加
  const applyTemplate = (name: string) =>
    setVars((vars) => {
      const have = new Set(vars.map((v) => v.key.trim().toUpperCase()));
      const add = TEMPLATES[name].filter((v) => !have.has(v.key.toUpperCase()));
      // 顺手清掉用户没填名字的空行，避免模板追加后表里夹着一堆空行
      return [...vars.filter((v) => v.key.trim()), ...add];
    });

  const fillSystemProxy = async () => {
    setError("");
    const proxy = await invoke<string | null>("system_proxy").catch(() => null);
    if (!proxy) {
      setError(t("profiles.noSystemProxy"));
      return;
    }
    setVars((vars) => {
      const next = [...vars.filter((v) => v.key.trim())];
      for (const key of ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"]) {
        const i = next.findIndex(
          (v) => v.key.trim().toUpperCase() === key,
        );
        if (i >= 0) next[i] = { ...next[i], value: proxy };
        else next.push({ ...V(key), value: proxy });
      }
      return next;
    });
  };

  const save = () => {
    if (!dirty || busy) return;
    // 同名变量后写覆盖先写，静默生效等于埋雷，先拦下来让用户改
    const seen = new Set<string>();
    const dup = new Set<string>();
    for (const p of store.profiles)
      for (const v of p.vars) {
        const k = `${p.id}|${v.key.trim().toUpperCase()}`;
        if (!v.key.trim()) continue;
        if (seen.has(k)) dup.add(v.key.trim());
        seen.add(k);
      }
    if (dup.size) {
      setError(t("profiles.dupKeys", { keys: [...dup].join(", ") }));
      return;
    }
    setBusy(true);
    setError("");
    saveProfiles(store)
      .then(() => loadProfiles()) // 重新拉一次，让 secret 的 hasValue 反映真实落盘状态
      .then((s) => {
        setStore(s);
        setDirty(false);
        setNotice(t("profiles.saved"));
        onChanged();
      })
      .catch((e) => setError(String(e)))
      .finally(() => setBusy(false));
  };

  return (
    <>
      <h2 className="settings-h2">{t("settings.profiles")}</h2>
      <p className="settings-sub">{t("profiles.sub")}</p>
      <p className="settings-hint">{t("profiles.newTabOnly")}</p>
      <p className="settings-hint">
        {store.encryptionAvailable
          ? t("profiles.encrypted")
          : t("profiles.plaintext")}
      </p>

      <div className="settings-section-title">{t("profiles.list")}</div>
      <div className="prof-chips">
        {store.profiles.map((p) => (
          <button
            key={p.id}
            className={"prof-chip" + (p.id === selected ? " selected" : "")}
            onClick={() => setSelected(p.id)}
          >
            {p.name.trim() || t("profiles.untitled")}
            {p.id === store.activeId && <span className="prof-chip-dot" />}
          </button>
        ))}
        <button className="prof-chip prof-chip-add" onClick={addProfile}>
          ＋ {t("profiles.new")}
        </button>
      </div>

      {!current ? (
        <p className="settings-hint">{t("profiles.empty")}</p>
      ) : (
        <>
          <input
            className="prof-name"
            value={current.name}
            spellCheck={false}
            placeholder={t("profiles.namePlaceholder")}
            onChange={(e) => patch((p) => ({ ...p, name: e.target.value }))}
          />

          <div className="settings-section-title">{t("profiles.templates")}</div>
          <div className="prof-tpl-row">
            <button className="prof-btn" onClick={() => applyTemplate("claude")}>
              {t("profiles.tplClaude")}
            </button>
            <button className="prof-btn" onClick={() => applyTemplate("codex")}>
              {t("profiles.tplCodex")}
            </button>
            <button className="prof-btn" onClick={() => applyTemplate("proxy")}>
              {t("profiles.tplProxy")}
            </button>
            <button className="prof-btn" onClick={fillSystemProxy}>
              {t("profiles.useSystemProxy")}
            </button>
          </div>

          <div className="settings-section-title">{t("profiles.vars")}</div>
          <div className="prof-vars">
            {current.vars.map((v, i) => (
              <div className="prof-var" key={i}>
                <input
                  className="prof-key"
                  value={v.key}
                  spellCheck={false}
                  placeholder={t("profiles.keyPlaceholder")}
                  onChange={(e) =>
                    setVars((vars) =>
                      vars.map((x, j) =>
                        j === i
                          ? {
                              ...x,
                              key: e.target.value,
                              // 后端是按 (配置组, 变量名) 找回已存密文的，改了名就对不上了。
                              // 与其让用户以为密钥还在、保存后变成空值，不如立刻显示
                              // "未设置"，把要重填这件事摆到明面上
                              hasValue: x.hasValue && x.key === e.target.value,
                              // 名字看着像密钥就自动上锁，用户仍可手动切回来
                              secret: x.hasValue
                                ? x.secret
                                : SECRET_HINT.test(e.target.value),
                            }
                          : x,
                      ),
                    )
                  }
                />
                <input
                  className="prof-value"
                  type={v.secret ? "password" : "text"}
                  value={v.value}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={
                    v.secret
                      ? v.hasValue
                        ? t("profiles.secretSet")
                        : t("profiles.secretUnset")
                      : t("profiles.valuePlaceholder")
                  }
                  onChange={(e) =>
                    setVars((vars) =>
                      vars.map((x, j) =>
                        j === i ? { ...x, value: e.target.value } : x,
                      ),
                    )
                  }
                />
                <button
                  className={"prof-lock" + (v.secret ? " on" : "")}
                  title={t("profiles.toggleSecret")}
                  onClick={() =>
                    setVars((vars) =>
                      vars.map((x, j) =>
                        j === i ? { ...x, secret: !x.secret } : x,
                      ),
                    )
                  }
                >
                  {v.secret ? "🔒" : "🔓"}
                </button>
                <button
                  className="prof-del"
                  title={t("profiles.removeVar")}
                  onClick={() =>
                    setVars((vars) => vars.filter((_, j) => j !== i))
                  }
                >
                  ×
                </button>
              </div>
            ))}
          </div>
          <button
            className="prof-btn"
            onClick={() => setVars((vars) => [...vars, V("", false)])}
          >
            ＋ {t("profiles.addVar")}
          </button>

          {error && <p className="prof-error">{error}</p>}
          {notice && !dirty && <p className="prof-ok">{notice}</p>}

          <div className="prof-actions">
            <button className="prof-btn prof-btn-danger" onClick={deleteProfile}>
              {confirmDelete ? t("profiles.deleteConfirm") : t("profiles.delete")}
            </button>
            <button
              className="prof-btn primary"
              disabled={!dirty || busy}
              onClick={save}
            >
              {busy ? t("profiles.saving") : t("profiles.save")}
            </button>
          </div>
        </>
      )}
    </>
  );
}
