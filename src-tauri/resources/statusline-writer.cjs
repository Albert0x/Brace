#!/usr/bin/env node
/*
 * Brace 用量采集器
 * ------------------------------------------------------------
 * 作为 Claude Code 的 statusLine command 运行。Claude Code 每次刷新底部状态栏，
 * 会把一份 JSON 通过 stdin 喂进来（含 model / context_window / rate_limits），
 * 这是官方运行时数据。本脚本把它写进共享缓存，供 Brace 读取显示。
 *
 * 关键点：rate_limits（5h/7d 额度）只在 API 响应后才随 stdin 传入，空闲刷新时没有。
 * 所以用"更新则覆盖、缺失则沿用缓存"的策略，保证多个会话看到一致的账号级额度。
 * 缓存格式：{ model, context_window, rate_limits, updated_at }
 */

const fs = require("fs");
const os = require("os");
const path = require("path");

const CACHE = path.join(os.homedir(), ".claude", "statusline-cache.json");

function readStdin() {
  try {
    const data = fs.readFileSync(0, "utf8"); // fd 0 = stdin
    return data ? JSON.parse(data) : null;
  } catch {
    return null;
  }
}

function loadCache() {
  try {
    return JSON.parse(fs.readFileSync(CACHE, "utf8"));
  } catch {
    return {};
  }
}

// resets_at 兼容 epoch 数字与 ISO 字符串，统一成 epoch 秒
function parseReset(r) {
  if (r == null) return null;
  if (typeof r === "number") return r;
  const t = Date.parse(String(r).replace("Z", "+00:00"));
  return isNaN(t) ? null : t / 1000;
}

// 传入的 rate_limits 是否比缓存更新：5h 窗口用量单调递增，
// resets_at 更大=新窗口；相同则用量更高的为新。否则视为空闲会话的陈旧值，不覆盖。
function fresher(incoming, cached) {
  if (!cached) return true;
  const ir = parseReset((incoming.five_hour || {}).resets_at);
  const cr = parseReset((cached.five_hour || {}).resets_at);
  if (cr == null) return true;
  if (ir == null) return false;
  if (ir > cr) return true;
  if (ir < cr) return false;
  const ip = (incoming.five_hour || {}).used_percentage || 0;
  const cp = (cached.five_hour || {}).used_percentage || 0;
  return ip >= cp;
}

function main() {
  const stdin = readStdin();
  const cache = loadCache();
  const prevRL = cache.rate_limits || null;

  let rateLimits = prevRL;
  if (stdin && stdin.rate_limits && fresher(stdin.rate_limits, prevRL)) {
    rateLimits = stdin.rate_limits;
  }

  const payload = {
    model: (stdin && stdin.model) || cache.model || null,
    context_window: (stdin && stdin.context_window) || null,
    rate_limits: rateLimits || null,
    updated_at: Date.now() / 1000,
  };

  // 原子写：先写临时文件再 rename，避免多会话并发写坏
  try {
    fs.mkdirSync(path.dirname(CACHE), { recursive: true });
    const tmp = CACHE + "." + process.pid + ".tmp";
    fs.writeFileSync(tmp, JSON.stringify(payload));
    fs.renameSync(tmp, CACHE);
  } catch {
    /* 缓存写失败不影响 claude 本身 */
  }

  // Brace 已在自己的状态栏显示用量，这里不再往 claude 原生底栏输出，
  // 避免同一份数据显示两遍。输出空串即可（数据已写入缓存供 Brace 读取）。
  process.stdout.write("");
}

main();
