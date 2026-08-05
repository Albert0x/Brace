import { useEffect, useState } from "react";

// localStorage 持久化的 state。
// 在此之前每个设置项都要手写一遍「useState 里读 localStorage + useEffect 里写回」，
// 同样的六行抄了十几遍，加一个开关就得再抄一遍。
function usePersisted<T>(
  key: string,
  initial: T,
  read: (raw: string) => T,
  write: (value: T) => string,
): [T, React.Dispatch<React.SetStateAction<T>>] {
  const [value, setValue] = useState<T>(() => {
    const raw = localStorage.getItem(key);
    return raw === null ? initial : read(raw);
  });
  useEffect(() => {
    try {
      localStorage.setItem(key, write(value));
    } catch {
      // 配额满之类的：本次会话照常生效，只是重启后回到默认值。
      // 一个界面偏好存不下不值得打断用户
    }
    // write/read 都是调用方每次渲染新建的箭头函数，进依赖会导致每次渲染都写一次盘
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, value]);
  return [value, setValue];
}

export const usePersistedString = (key: string, initial: string) =>
  usePersisted(
    key,
    initial,
    (raw) => raw,
    (v) => v,
  );

export const usePersistedNumber = (key: string, initial: number) =>
  usePersisted(key, initial, (raw) => Number(raw) || initial, String);

// 布尔存 "1"/"0"——沿用已经写进老用户 localStorage 的格式，别让升级的人设置被重置
export const usePersistedBool = (key: string, initial: boolean) =>
  usePersisted(
    key,
    initial,
    (raw) => raw === "1",
    (v) => (v ? "1" : "0"),
  );
