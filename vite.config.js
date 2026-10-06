import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri 用固定端口，端口被占用时直接失败而不是自动换端口，
// 否则 tauri.conf.json 里的 devUrl 会对不上。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5200,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    target: 'chrome110',
    sourcemap: false,
  },
});
