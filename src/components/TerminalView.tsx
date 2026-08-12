import { useEffect, useRef, useState } from "react";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useT } from "../i18n";
import "@xterm/xterm/css/xterm.css";

const ANSI_COLORS: ITheme = {
  black: "#25272b",
  red: "#ff6b72",
  green: "#8bd49c",
  yellow: "#e5c07b",
  blue: "#7ab7ff",
  magenta: "#c99cff",
  cyan: "#69d5d0",
  white: "#d8dee9",
  brightBlack: "#697180",
  brightRed: "#ff858b",
  brightGreen: "#a6e3b5",
  brightYellow: "#f0d49a",
  brightBlue: "#9ac8ff",
  brightMagenta: "#d9b7ff",
  brightCyan: "#8be2de",
  brightWhite: "#f5f7fa",
};

interface Props {
  sessionId: string;
  active: boolean;
  onCwd: (sessionId: string, path: string) => void;
  termTheme: ITheme;
  initialCwd: string;
  fontSize: number;
  cursorBlink: boolean;
  webgl: boolean;
  shellPath: string;
  shellType: string;
  onRegisterSearch: (id: string, addon: SearchAddon) => void;
  onUnregisterSearch: (id: string) => void;
  debugInput: boolean;
  startupCommand?: "claude" | "codex";
  activeAgent: string;
  onNewTerminal: () => void;
  onNewConversation: () => void;
}

// 终端视图：一个实例对应后端一个 pty 会话
export default function TerminalView({
  sessionId,
  active,
  onCwd,
  termTheme,
  initialCwd,
  fontSize,
  cursorBlink,
  webgl,
  shellPath,
  shellType,
  onRegisterSearch,
  onUnregisterSearch,
  debugInput,
  startupCommand,
  activeAgent,
  onNewTerminal,
  onNewConversation,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [menuAgent, setMenuAgent] = useState("");
  const [menuHasSelection, setMenuHasSelection] = useState(false);
  const t = useT();

  // 输入诊断。开关走 ref 读：让 effect 依赖 debugInput 会重建终端，而重建终端
  // 等于杀掉 PTY 会话——为了打个日志把用户的 shell 干掉，那就本末倒置了。
  // 监听器常驻，关闭时第一行就返回，开销可以忽略
  const debugRef = useRef(debugInput);
  debugRef.current = debugInput;
  const debugBuf = useRef<string[]>([]);

  const copySelection = () => {
    const selection = termRef.current?.getSelection();
    if (!selection) return;
    navigator.clipboard.writeText(selection).catch(() => {});
    termRef.current?.focus();
  };

  const pasteIntoTerminal = () => {
    navigator.clipboard
      .readText()
      .then((text) => {
        if (text) termRef.current?.paste(text);
        termRef.current?.focus();
      })
      .catch(() => termRef.current?.focus());
  };

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setMenuAgent(activeAgent);
    setMenuHasSelection(termRef.current?.hasSelection() ?? false);
    setMenu({ x: e.clientX, y: e.clientY });
    invoke<{ agent: string }>("usage_stats", { sessionId })
      .then((stats) => setMenuAgent(stats.agent))
      .catch(() => {});
  };

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [menu]);

  useEffect(() => {
    if (!containerRef.current) return;
    let disposed = false;

    const term = new Terminal({
      cursorBlink,
      allowTransparency: true,
      fontFamily: "Menlo, Monaco, 'Cascadia Mono', monospace",
      fontSize,
      fontWeight: "500",
      fontWeightBold: "600",
      lineHeight: 1.2,
      letterSpacing: 0,
      theme: { ...ANSI_COLORS, ...termTheme },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    const search = new SearchAddon();
    term.loadAddon(search);

    // URL 可点击，用系统浏览器打开
    term.loadAddon(
      new WebLinksAddon((_e, uri) => {
        openUrl(uri).catch(console.error);
      }),
    );

    term.open(containerRef.current);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;

    // WebGL 渲染：设置里开启时才懒加载（不进初始 bundle）；关闭则用 xterm 自带 canvas 渲染
    if (webgl) {
      import("@xterm/addon-webgl")
        .then(({ WebglAddon }) => {
          if (disposed) return;
          const addon = new WebglAddon();
          addon.onContextLoss(() => addon.dispose());
          term.loadAddon(addon);
        })
        .catch(() => {});
    }

    onRegisterSearch(sessionId, search);

    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      if (e.ctrlKey && e.shiftKey && e.code === "KeyC") {
        const sel = term.getSelection();
        if (sel) {
          navigator.clipboard.writeText(sel).catch(() => {});
          return false;
        }
      }
      if (e.ctrlKey && e.shiftKey && e.code === "KeyV") {
        // 不手动粘贴——交给 WebView 原生 paste 事件（xterm textarea 处理，带 bracketed paste）。
        // 这里仅 return false，阻止 xterm 把 Ctrl+Shift+V 当控制字符发出，避免粘贴两遍。
        return false;
      }
      return true;
    });

    term.parser.registerOscHandler(9, (data) => {
      if (data.startsWith("9;")) {
        onCwd(sessionId, data.slice(2));
        return true;
      }
      return false;
    });

    // 必须等监听器真正注册完再建 PTY，否则 shell 启动瞬间的输出可能在监听器就位前就已发出而丢失
    const unlistenPromise = listen<{ id: string; data: string }>("pty-output", (e) => {
      if (e.payload.id === sessionId) term.write(e.payload.data);
    });
    unlistenPromise.then(() => {
      if (disposed) return;
      invoke("pty_create", {
        id: sessionId,
        rows: term.rows,
        cols: term.cols,
        cwd: initialCwd,
        shellPath,
        shellType,
      }).then(() => {
        if (startupCommand === "claude" || startupCommand === "codex") {
          return invoke("pty_write", { id: sessionId, data: `${startupCommand}\r` });
        }
      }).catch((e) => {
        // 起不来就直接写在终端里。以前只 console.error，用户看到的是一个
        // 一动不动的黑框，完全不知道发生了什么——恢复出来的标签指向一个
        // 已经卸载掉的 shell 时尤其容易撞上
        term.write(`\r\n\x1b[31m${t("term.spawnFailed", { e: String(e) })}\x1b[0m\r\n`);
      });
    });

    // 诊断：把按键、输入法组合事件、以及最终发往 PTY 的字节按时间顺序记下来。
    // 只看最终字节不够定位 IME 问题——得能还原事件先后和各时刻 textarea 的内容
    const stamp = () => {
      const d = new Date();
      return (
        d.toTimeString().slice(0, 8) +
        "." +
        String(d.getMilliseconds()).padStart(3, "0")
      );
    };
    const rec = (tag: string, detail: string) => {
      if (!debugRef.current) return;
      debugBuf.current.push(`${stamp()} ${tag} ${detail}`);
    };
    const ta = term.textarea;
    const taValue = () => JSON.stringify(ta?.value ?? "");
    const onComposition = (e: Event) => {
      const data = (e as CompositionEvent).data ?? "";
      rec(e.type, `data=${JSON.stringify(data)} textarea=${taValue()}`);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      // keyCode 229 是「输入法正在处理」的标记，xterm 对它有专门分支，
      // 重复输入的两个嫌疑点都在那条路径上，所以必须记下来
      rec(
        "keydown",
        `key=${JSON.stringify(e.key)} code=${e.code} keyCode=${e.keyCode} textarea=${taValue()}`,
      );
    };
    ta?.addEventListener("compositionstart", onComposition);
    ta?.addEventListener("compositionupdate", onComposition);
    ta?.addEventListener("compositionend", onComposition);
    ta?.addEventListener("keydown", onKeyDown);

    // 攒一秒写一次盘：keydown 很密，每条一次 IPC 会把通道占满
    const flushDebug = () => {
      const lines = debugBuf.current.splice(0);
      if (lines.length) invoke("append_debug_log", { lines }).catch(() => {});
    };
    const debugTimer = setInterval(flushDebug, 1000);

    term.onData((data) => {
      rec("onData", JSON.stringify(data));
      invoke("pty_write", { id: sessionId, data }).catch(console.error);
    });

    const syncSize = () => {
      fit.fit();
      invoke("pty_resize", {
        id: sessionId,
        rows: term.rows,
        cols: term.cols,
      }).catch(console.error);
    };
    window.addEventListener("resize", syncSize);

    return () => {
      disposed = true;
      clearInterval(debugTimer);
      flushDebug(); // 关标签前把没写完的补上
      ta?.removeEventListener("compositionstart", onComposition);
      ta?.removeEventListener("compositionupdate", onComposition);
      ta?.removeEventListener("compositionend", onComposition);
      ta?.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("resize", syncSize);
      unlistenPromise.then((f) => f());
      onUnregisterSearch(sessionId);
      invoke("pty_close", { id: sessionId }).catch(() => {});
      term.dispose();
    };
  }, [sessionId]);

  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = { ...ANSI_COLORS, ...termTheme };
  }, [termTheme]);

  // 光标闪烁开关实时生效
  useEffect(() => {
    if (termRef.current) termRef.current.options.cursorBlink = cursorBlink;
  }, [cursorBlink]);

  // 字体大小变化 → 应用并重新适配
  useEffect(() => {
    const t = termRef.current;
    if (!t) return;
    t.options.fontSize = fontSize;
    fitRef.current?.fit();
    invoke("pty_resize", { id: sessionId, rows: t.rows, cols: t.cols }).catch(
      console.error,
    );
  }, [fontSize]);

  useEffect(() => {
    if (!active || !termRef.current || !fitRef.current) return;
    requestAnimationFrame(() => {
      fitRef.current?.fit();
      const t = termRef.current;
      if (t) {
        invoke("pty_resize", { id: sessionId, rows: t.rows, cols: t.cols }).catch(
          console.error,
        );
        t.focus();
      }
    });
  }, [active, sessionId]);

  return (
    <div
      className="terminal-view"
      style={{ display: active ? "block" : "none" }}
      onContextMenu={handleContextMenu}
    >
      <div ref={containerRef} className="terminal-host" />
      {menu && (
        <div
          className="ctx-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <button className="ctx-item" type="button" disabled>
            <span className="ctx-leading-icon" aria-hidden="true">✂︎</span>
            <span>{t("ctx.cut")}</span>
          </button>
          <button
            className="ctx-item"
            type="button"
            disabled={!menuHasSelection}
            onClick={() => { copySelection(); setMenu(null); }}
          >
            <span className="ctx-leading-icon" aria-hidden="true">▧</span>
            <span>{t("ctx.copy")}</span>
          </button>
          <button
            className="ctx-item"
            type="button"
            onClick={() => { pasteIntoTerminal(); setMenu(null); }}
          >
            <span className="ctx-leading-icon" aria-hidden="true">▣</span>
            <span>{t("ctx.paste")}</span>
          </button>
          <div className="ctx-sep" />
          <button className="ctx-item" type="button" onClick={() => { onNewTerminal(); setMenu(null); }}>
            <span className="ctx-leading-icon" aria-hidden="true">▣</span>
            <span>{t("ctx.newTerminal")}</span>
          </button>
          {(menuAgent === "claude" || menuAgent === "codex") && (
            <button className="ctx-item" type="button" onClick={() => { onNewConversation(); setMenu(null); }}>
              <span className="ctx-leading-icon" aria-hidden="true">✦</span>
              <span>
                {t("ctx.newConversation", {
                  agent: menuAgent === "claude" ? "Claude" : "Codex",
                })}
              </span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}
