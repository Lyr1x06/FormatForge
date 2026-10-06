import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { fileURLToPath } from 'node:url';

/**
 * UI 验证台专用配置，与 vite.config.js 相互独立。
 * 起法：npx vite --config vite.mock.config.js  然后开 http://localhost:5210
 */
export default defineConfig({
  root: 'mock',
  plugins: [react()],
  clearScreen: false,
  server: { port: 5210, strictPort: true },
  resolve: {
    alias: {
      // 浏览器里没有 Tauri 运行时，这个模块要换成空实现
      '@tauri-apps/api/webview': fileURLToPath(new URL('./mock/tauri-stub.js', import.meta.url)),
    },
  },
});
