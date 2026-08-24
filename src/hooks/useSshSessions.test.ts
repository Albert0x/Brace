import { describe, expect, it } from "vitest";
import { emptySshSession, sshLabel, type SshSession } from "./useSshSessions";

const session = (over: Partial<SshSession> = {}): SshSession => ({
  ...emptySshSession(),
  host: "example.com",
  ...over,
});

// 这套规则在 Rust 侧的 ssh::display_name 里有一份等价实现（后端拼命令时要用）。
// 两份实现同一套规则，最容易悄悄分叉——两边都有测试盯着才不会。
describe("sshLabel", () => {
  it("有名字就用名字", () => {
    expect(sshLabel(session({ name: "prod-db", user: "root" }))).toBe("prod-db");
  });

  it("没名字时退回 user@host", () => {
    expect(sshLabel(session({ user: "root" }))).toBe("root@example.com");
  });

  it("连用户名都没填时只显示主机", () => {
    // 用户名留空是有意义的——交给 ~/.ssh/config 决定，所以标签也不该编一个出来
    expect(sshLabel(session())).toBe("example.com");
  });

  it("忽略字段两侧的空白", () => {
    // 从别处复制粘贴过来的值经常带空格
    expect(sshLabel(session({ name: "  prod  " }))).toBe("prod");
    expect(sshLabel(session({ host: " h ", user: " u " }))).toBe("u@h");
  });

  it("名字只有空白时视为没填", () => {
    expect(sshLabel(session({ name: "   ", user: "root" }))).toBe(
      "root@example.com",
    );
  });
});

describe("emptySshSession", () => {
  it("端口默认为 0，表示交给 ssh 自己决定", () => {
    // 这里若默认填 22，就会往命令行里塞一个 -p 22，把 ~/.ssh/config 里的
    // Port 设置盖掉——留 0 才是「没指定」
    expect(emptySshSession().port).toBe(0);
  });

  it("每次生成不同的 id", () => {
    expect(emptySshSession().id).not.toBe(emptySshSession().id);
  });

  it("不含任何密码字段", () => {
    // 密码由 ssh 自己在 PTY 里问。这条测试是为了让「加个密码字段吧」
    // 这种想法在 CI 上就被拦住
    expect(Object.keys(emptySshSession())).not.toContain("password");
  });
});
