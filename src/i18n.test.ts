import { describe, expect, it } from "vitest";
import { createT, LANGS, MESSAGES } from "./i18n";

describe("MESSAGES 完整性", () => {
  // 漏翻译的表现是界面上突然冒出一句英文，而且只有切到中文的人才看得见。
  // 与其等用户报，不如让 CI 拦下来
  it("每种语言的 key 集合都和英文一致", () => {
    const en = Object.keys(MESSAGES.en).sort();
    for (const { code } of LANGS) {
      if (code === "en") continue;
      const keys = Object.keys(MESSAGES[code]).sort();
      expect(keys, `${code} 的 key 与 en 不一致`).toEqual(en);
    }
  });

  it("LANGS 里的每种语言都有对应的文案表", () => {
    for (const { code } of LANGS) {
      expect(MESSAGES[code], `缺少 ${code} 的文案表`).toBeTruthy();
    }
  });
});

describe("createT", () => {
  it("按语言取对应文案", () => {
    const en = createT("en");
    const zh = createT("zh");
    expect(en("win.close")).toBe(MESSAGES.en["win.close"]);
    expect(zh("win.close")).toBe(MESSAGES.zh["win.close"]);
  });

  it("两种语言都没有的 key 原样返回，不返回 undefined", () => {
    // 界面上看到一个 key 名很难看，但比 "undefined" 或直接崩掉好定位
    expect(createT("zh")("nope.not.a.key")).toBe("nope.not.a.key");
  });

  it("替换单个占位符", () => {
    const t = createT("en");
    expect(t("term.spawnFailed", { e: "boom" })).toBe(
      "Failed to start shell: boom",
    );
  });

  it("数字参数会转成字符串", () => {
    const t = createT("en");
    expect(t("status.terminals", { n: 3 })).toBe("3 terminal(s)");
  });

  it("同一条文案里的多个不同占位符都会被替换", () => {
    const t = createT("en");
    const out = t("tree.enterDir", { path: "C:\\tmp" });
    expect(out).toContain("C:\\tmp");
    expect(out).not.toContain("{path}");
  });

  it("不传 params 时占位符原样保留", () => {
    expect(createT("en")("term.spawnFailed")).toContain("{e}");
  });

  it("多余的 params 不会报错", () => {
    expect(() =>
      createT("en")("win.close", { unused: "x" }),
    ).not.toThrow();
  });
});
