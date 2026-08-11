import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// 测试配置和 vite.config.ts 分开：那边的 manualChunks / Tauri devServer 设置
// 跟跑测试没关系，混在一起只会让两边都难读。
export default defineConfig({
  plugins: [react()],
  test: {
    // hooks 要读 localStorage、注册 window 事件监听，需要 DOM
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test/setup.ts"],
    // 不开 globals：显式 import 才知道 describe/it 是哪来的，也不用改 tsconfig types
    globals: false,
    restoreMocks: true,
  },
});
