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
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const t = useT();

  const paste = () => {
    navigator.clipboard
      .readText()
      .then((text) => {
        if (text) {
          // 用 xterm 的 paste 而非裸 pty_write：自动带 bracketed paste 包裹，
          // 让 claude/vim 等能区分"粘贴"与"手动键入"（多行不会被逐行执行），
          // 并把焦点拉回终端，粘完能直接回车。
          termRef.current?.paste(text);
          termRef.current?.focus();
        }
      })
      .catch(() => {});
  };

  const copySelection = () => {
    const sel = termRef.current?.getSelection();
    if (sel) {
      navigator.clipboard.writeText(sel).catch(() => {});
      termRef.current?.clearSelection();
    }
  };

  const handleContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    setMenu({ x: e.clientX, y: e.clientY });
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
      fontFamily: "'Cascadia Mono', Consolas, 'Courier New', monospace",
      fontSize,
      theme: termTheme,
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

    invoke("pty_create", {
      id: sessionId,
      rows: term.rows,
      cols: term.cols,
      cwd: initialCwd,
      shellPath,
      shellType,
    }).catch(console.error);

    term.onData((data) => {
      invoke("pty_write", { id: sessionId, data }).catch(console.error);
    });

    const unlisten = listen<{ id: string; data: string }>("pty-output", (e) => {
      if (e.payload.id === sessionId) term.write(e.payload.data);
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
      window.removeEventListener("resize", syncSize);
      unlisten.then((f) => f());
      onUnregisterSearch(sessionId);
      invoke("pty_close", { id: sessionId }).catch(() => {});
      term.dispose();
    };
  }, [sessionId]);

  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = termTheme;
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
          <div className="ctx-item" onClick={() => { copySelection(); setMenu(null); }}>
            <span>{t("ctx.copy")}</span>
            <span className="ctx-key">Ctrl+Shift+C</span>
          </div>
          <div className="ctx-item" onClick={() => { paste(); setMenu(null); }}>
            <span>{t("ctx.paste")}</span>
            <span className="ctx-key">Ctrl+Shift+V</span>
          </div>
          <div className="ctx-sep" />
          <div className="ctx-item" onClick={() => { termRef.current?.selectAll(); setMenu(null); }}>
            <span>{t("ctx.selectAll")}</span>
          </div>
          <div className="ctx-item" onClick={() => { termRef.current?.clear(); setMenu(null); }}>
            <span>{t("ctx.clear")}</span>
          </div>
        </div>
      )}
    </div>
  );
}
