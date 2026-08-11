import { describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import {
  usePersistedBool,
  usePersistedNumber,
  usePersistedString,
} from "./usePersisted";

describe("usePersistedString", () => {
  it("没存过时用初始值", () => {
    const { result } = renderHook(() => usePersistedString("k", "def"));
    expect(result.current[0]).toBe("def");
  });

  it("存过就读回存的值", () => {
    localStorage.setItem("k", "saved");
    const { result } = renderHook(() => usePersistedString("k", "def"));
    expect(result.current[0]).toBe("saved");
  });

  it("空字符串是合法值，不会被当成没存过", () => {
    // 判断用的是 raw === null 而不是 falsy，所以「用户清空了输入框」这个状态能存下来
    localStorage.setItem("k", "");
    const { result } = renderHook(() => usePersistedString("k", "def"));
    expect(result.current[0]).toBe("");
  });

  it("改值后写回 localStorage", () => {
    const { result } = renderHook(() => usePersistedString("k", "def"));
    act(() => result.current[1]("next"));
    expect(result.current[0]).toBe("next");
    expect(localStorage.getItem("k")).toBe("next");
  });
});

describe("usePersistedBool", () => {
  it('存的是 "1"/"0" 而不是 true/false', () => {
    // 老用户的 localStorage 里已经是这个格式，改格式等于把他们的设置重置一遍
    const { result } = renderHook(() => usePersistedBool("b", false));
    act(() => result.current[1](true));
    expect(localStorage.getItem("b")).toBe("1");
    act(() => result.current[1](false));
    expect(localStorage.getItem("b")).toBe("0");
  });

  it('"1" 读成 true', () => {
    localStorage.setItem("b", "1");
    const { result } = renderHook(() => usePersistedBool("b", false));
    expect(result.current[0]).toBe(true);
  });

  it("非 \"1\" 的任何内容都读成 false", () => {
    for (const raw of ["0", "true", "yes", "", "garbage"]) {
      localStorage.setItem("b", raw);
      const { result } = renderHook(() => usePersistedBool("b", true));
      expect(result.current[0], `raw=${JSON.stringify(raw)}`).toBe(false);
    }
  });
});

describe("usePersistedNumber", () => {
  it("正常数字读回原值", () => {
    localStorage.setItem("n", "18");
    const { result } = renderHook(() => usePersistedNumber("n", 14));
    expect(result.current[0]).toBe(18);
  });

  it("小数不丢精度", () => {
    localStorage.setItem("n", "0.35");
    const { result } = renderHook(() => usePersistedNumber("n", 0.5));
    expect(result.current[0]).toBe(0.35);
  });

  it("读不成数字的内容回退初始值", () => {
    localStorage.setItem("n", "not-a-number");
    const { result } = renderHook(() => usePersistedNumber("n", 14));
    expect(result.current[0]).toBe(14);
  });

  // ⚠️ 锁的是「当前行为」，不是「期望行为」。
  // 实现是 `Number(raw) || initial`，而 0 是 falsy，所以存进去的 0 读回来会变成默认值。
  // 目前三个使用处（ht-fontsize 8-28、ht-zoom、ht-overlay 0.2-0.95）都取不到 0，
  // 所以还没有人踩到。但以后任何一个允许 0 的数值设置都会中招，
  // 症状是「我设成 0，重启又变回默认值」，极难排查。
  // 这条测试的作用是：真去修的时候它会红，提醒你确认这是有意的行为变更。
  it("存 0 会读回初始值 —— Number(raw) || initial 的已知陷阱", () => {
    localStorage.setItem("n", "0");
    const { result } = renderHook(() => usePersistedNumber("n", 14));
    expect(result.current[0]).toBe(14);
  });

  it("改值后写回 localStorage", () => {
    const { result } = renderHook(() => usePersistedNumber("n", 14));
    act(() => result.current[1](20));
    expect(localStorage.getItem("n")).toBe("20");
  });

  it("支持函数式更新", () => {
    const { result } = renderHook(() => usePersistedNumber("n", 14));
    act(() => result.current[1]((f) => f + 2));
    expect(result.current[0]).toBe(16);
  });
});
