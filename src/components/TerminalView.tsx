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
  scrollback: number;
  shellPath: string;
  shellType: string;
  args: string[];
  onRegisterSearch: (id: string, addon: SearchAddon) => void;
  onUnregisterSearch: (id: string) => void;
  debugInput: boolean;
  onExit: (sessionId: string, code: number | null) => void;
  onRestarted: (sessionId: string) => void;
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
  scrollback,
  shellPath,
  shellType,
  args,
  onRegisterSearch,
  onUnregisterSearch,
  debugInput,
  onExit,
  onRestarted,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // 进程是否已经退出。走 ref 是因为 onData 回调建立在首次渲染的闭包里，
  // 读 state 只会读到那一刻的旧值
  const exitedRef = useRef(false);
  // 退出前最后上报的目录。重启时回到这里，而不是把用户丢回 home
  const lastCwdRef = useRef(initialCwd);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const t = useT();

  // 输入诊断。开关走 ref 读：让 effect 依赖 debugInput 会重建终端，而重建终端
  // 等于杀掉 PTY 会话——为了打个日志把用户的 shell 干掉，那就本末倒置了。
  // 监听器常驻，关闭时第一行就返回，开销可以忽略
  const debugRef = useRef(debugInput);
  debugRef.current = debugInput;
  const debugBuf = useRef<string[]>([]);

  // 剪贴板里是图片时给一行提示。终端本身没有显示图片的能力，
  // 但至少不能像以前那样「按了粘贴，什么都没发生，也不说为什么」
  const noticeImageInClipboard = () => {
    termRef.current?.write(
      `\r\n\x1b[33m${t("term.imageInClipboard")}\x1b[0m\r\n`,
    );
  };

  const paste = () => {
    navigator.clipboard
      .readText()
      .then(async (text) => {
        if (text) {
          // 用 xterm 的 paste 而非裸 pty_write：自动带 bracketed paste 包裹，
          // 让 claude/vim 等能区分"粘贴"与"手动键入"（多行不会被逐行执行），
          // 并把焦点拉回终端，粘完能直接回车。
          termRef.current?.paste(text);
          termRef.current?.focus();
          return;
        }
        // 文本为空有两种可能：剪贴板真的空着，或者里面是图片——
        // readText() 对图片就是返回空串。区分一下，后者给提示
        try {
          const items = await navigator.clipboard.read();
          if (items.some((it) => it.types.some((ty) => ty.startsWith("image/"))))
            noticeImageInClipboard();
        } catch {
          // 读剪贴板要权限，拿不到就算了。宁可不提示也不要误报
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
      scrollback,
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
      // Ctrl+V 和 Ctrl+Shift+V 都是粘贴。不手动粘贴——交给 WebView 原生 paste 事件
      // （xterm textarea 处理，带 bracketed paste）。这里只 return false，
      // 阻止 xterm 把它当控制字符发出去。
      //
      // Ctrl+V 以前没拦，xterm 会先发一个 ^V，紧接着原生 paste 再送来包好的内容。
      // readline 把 ^V 当 quoted-insert，于是 ESC[200~ 的 ESC 被当普通字符插进命令行，
      // 粘贴进 Git Bash 的命令就变成 $'\E[200~gh' 这种东西，末尾还多一个 ~。
      // 排除 Alt：Ctrl+Alt 在部分键盘布局上是 AltGr，得留给它打字
      if (e.ctrlKey && !e.altKey && e.code === "KeyV") {
        return false;
      }
      return true;
    });

    term.parser.registerOscHandler(9, (data) => {
      if (data.startsWith("9;")) {
        const path = data.slice(2);
        lastCwdRef.current = path;
        onCwd(sessionId, path);
        return true;
      }
      return false;
    });

    // 原地重启：复用同一个 sessionId 和同一个 xterm 实例，
    // 所以之前的输出全都留在屏幕上——那正是排查「它为什么挂了」时要看的东西
    const restart = () => {
      invoke("pty_create", {
        id: sessionId,
        rows: term.rows,
        cols: term.cols,
        cwd: lastCwdRef.current || initialCwd,
        shellPath,
        shellType,
        args,
      })
        .then(() => {
          exitedRef.current = false;
          onRestarted(sessionId);
          term.focus();
        })
        .catch((e) => {
          term.write(
            `\r\n\x1b[31m${t("term.spawnFailed", { e: String(e) })}\x1b[0m\r\n`,
          );
        });
    };

    // 必须等监听器真正注册完再建 PTY，否则 shell 启动瞬间的输出可能在监听器就位前就已发出而丢失
    const unlistenPromise = listen<{ id: string; data: string }>("pty-output", (e) => {
      if (e.payload.id === sessionId) term.write(e.payload.data);
    });

    // shell 退出。后端一直在发这个事件，但以前前端没有任何人在听，
    // 于是用户面对的是一个能打字、却永远不回话的黑框
    const unlistenExitPromise = listen<{ id: string; code: number | null }>(
      "pty-exit",
      (e) => {
        if (e.payload.id !== sessionId) return;
        exitedRef.current = true;
        const code = e.payload.code ?? null;
        term.write(
          `\r\n\x1b[33m${t("term.exited", {
            code: code === null ? "?" : String(code),
          })}\x1b[0m\r\n`,
        );
        onExit(sessionId, code);
      },
    );
    unlistenPromise.then(() => {
      if (disposed) return;
      invoke("pty_create", {
        id: sessionId,
        rows: term.rows,
        cols: term.cols,
        cwd: initialCwd,
        shellPath,
        shellType,
        args,
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
    // Ctrl+V / Ctrl+Shift+V 是交给 WebView 原生 paste 事件处理的（见上面的按键处理器），
    // 走不到上面那个 paste()。图片在这条路径上同样是「按了没反应」，得单独拦。
    // ClipboardEvent 自带 clipboardData，不需要剪贴板读取权限
    const onPasteEvent = (e: ClipboardEvent) => {
      const dt = e.clipboardData;
      if (!dt) return;
      const types = Array.from(dt.types);
      if (
        !types.includes("text/plain") &&
        types.some((ty) => ty.startsWith("image/"))
      ) {
        noticeImageInClipboard();
      }
    };

    ta?.addEventListener("compositionstart", onComposition);
    ta?.addEventListener("compositionupdate", onComposition);
    ta?.addEventListener("compositionend", onComposition);
    ta?.addEventListener("keydown", onKeyDown);
    ta?.addEventListener("paste", onPasteEvent);

    // 攒一秒写一次盘：keydown 很密，每条一次 IPC 会把通道占满
    const flushDebug = () => {
      const lines = debugBuf.current.splice(0);
      if (lines.length) invoke("append_debug_log", { lines }).catch(() => {});
    };
    const debugTimer = setInterval(flushDebug, 1000);

    term.onData((data) => {
      rec("onData", JSON.stringify(data));
      // 进程已经没了，不再往死管道里灌数据。回车 = 原地重启（提示里写了），
      // 其余按键直接吞掉——总好过让用户对着一个不会有任何反应的黑框空敲
      if (exitedRef.current) {
        if (data === "\r") restart();
        return;
      }
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
      ta?.removeEventListener("paste", onPasteEvent);
      window.removeEventListener("resize", syncSize);
      unlistenPromise.then((f) => f());
      unlistenExitPromise.then((f) => f());
      onUnregisterSearch(sessionId);
      invoke("pty_close", { id: sessionId }).catch(() => {});
      term.dispose();
    };
    // 依赖只留 sessionId 是有意的：这个 effect 重跑等于 dispose 终端 + kill PTY 会话，
    // 为了一个主题色或字号变化就把用户的 shell 干掉完全说不通。其余 props 各自有
    // 单独的 effect 做增量同步（见下方），只有 webgl 例外——它标注了「对新终端生效」
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  useEffect(() => {
    if (termRef.current) termRef.current.options.theme = termTheme;
  }, [termTheme]);

  // 光标闪烁开关实时生效
  useEffect(() => {
    if (termRef.current) termRef.current.options.cursorBlink = cursorBlink;
  }, [cursorBlink]);

  // 回看行数实时生效。调小时 xterm 会立刻裁掉超出的历史，所以设置面板里
  // 要写清楚「调小会丢弃已有回看」，别让用户手一滑就把上文弄没了
  useEffect(() => {
    if (termRef.current) termRef.current.options.scrollback = scrollback;
  }, [scrollback]);

  // 字体大小变化 → 应用并重新适配
  useEffect(() => {
    const t = termRef.current;
    if (!t) return;
    t.options.fontSize = fontSize;
    fitRef.current?.fit();
    invoke("pty_resize", { id: sessionId, rows: t.rows, cols: t.cols }).catch(
      console.error,
    );
    // sessionId 在组件生命周期内不变（它就是 key），列进依赖只是让 lint 满意，不改行为
  }, [fontSize, sessionId]);

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
