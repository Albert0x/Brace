import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface ProfileVar {
  key: string;
  value: string; // secret 的值后端不回传，这里为空就表示"没改"
  secret: boolean;
  hasValue: boolean;
}

export interface Profile {
  id: string;
  name: string;
  vars: ProfileVar[];
}

export interface ProfileStore {
  profiles: Profile[];
  activeId: string;
  encryptionAvailable: boolean;
}

export const EMPTY_STORE: ProfileStore = {
  profiles: [],
  activeId: "",
  encryptionAvailable: false,
};

export const loadProfiles = () =>
  invoke<ProfileStore>("load_profiles").catch(() => EMPTY_STORE);

// 保存时只回传后端认识的字段，hasValue 是纯展示用的
export const saveProfiles = (store: ProfileStore) =>
  invoke("save_profiles", {
    store: {
      activeId: store.activeId,
      profiles: store.profiles.map((p) => ({
        id: p.id,
        name: p.name,
        vars: p.vars
          .filter((v) => v.key.trim())
          .map((v) => ({ key: v.key.trim(), value: v.value, secret: v.secret })),
      })),
    },
  });

// 环境变量配置组：状态栏显示当前生效的那组，点开可快速切换。
// 注入发生在后端 pty_create，所以切换只影响之后新建的标签
export function useProfiles() {
  const [store, setStore] = useState<ProfileStore>(EMPTY_STORE);
  const [menuOpen, setMenuOpen] = useState(false);

  const refresh = useCallback(() => {
    loadProfiles().then(setStore);
  }, []);
  useEffect(refresh, [refresh]);

  useEffect(() => {
    if (!menuOpen) return;
    const close = () => setMenuOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menuOpen]);

  const switchTo = (id: string) => {
    const next = { ...store, activeId: id };
    setStore(next); // 先动 UI，失败再回滚，避免点一下毫无反应
    saveProfiles(next)
      .then(refresh)
      .catch(() => setStore(store));
    setMenuOpen(false);
  };

  const active = store.profiles.find((p) => p.id === store.activeId);

  return { store, active, switchTo, refresh, menuOpen, setMenuOpen };
}
