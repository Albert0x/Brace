import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  // src-tauri 是 Rust（有自己的 clippy），dist/coverage 是产物
  { ignores: ["dist", "coverage", "src-tauri", ".pnpm-store", "web"] },

  js.configs.recommended,
  tseslint.configs.recommended,
  reactHooks.configs.flat.recommended,

  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: globals.browser,
    },
    rules: {
      // 下划线前缀表示「解构出来只为了丢掉它」，比如从 map 里剔除一个 key
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      // 空 catch 是有意的：存档坏了、剪贴板没权限这类场景，兜底逻辑写在外面，
      // catch 里只需要「别崩」。这些地方都有注释说明，不是吞异常
      "no-empty": ["error", { allowEmptyCatch: true }],
      // 依赖数组不全在这个项目里有多处刻意为之（重建终端等于杀掉 PTY 会话），
      // 所以降级为警告；真正违反的地方就地写 disable 注释并说明原因
      "react-hooks/exhaustive-deps": "warn",

      // ↓ 以下两条是 eslint-plugin-react-hooks v7 新增的 React Compiler 就绪度检查。
      // 它们指出的风险是真的，但只在并发渲染（Suspense / startTransition）下才会显形，
      // 而这个项目一处都没用。当前共 11 处违反、横跨 7 个文件，全是同两个惯用法：
      //   refs             —— 渲染期给 ref 赋最新值，供 effect / 事件回调读
      //   set-state-in-effect —— effect 里同步 setState 做数据加载
      // 改它们要动时序，是一次独立的重构，不该混在「把 lint 装起来」这件事里。
      // 暂设为 warn：问题保持可见，但不阻塞。详见 docs/ROADMAP.md 的待评估项。
      "react-hooks/refs": "warn",
      "react-hooks/set-state-in-effect": "warn",
    },
  },

  // 配置文件和测试跑在 node 环境
  {
    files: ["*.config.{js,ts}", "src/**/*.test.{ts,tsx}"],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
  },
);
