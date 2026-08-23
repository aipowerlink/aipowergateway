import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// 管理网页构建：产物输出到 dist/（组长端 HTTP 服务托管）
export default defineConfig({
  plugins: [react()],
  base: './',
  build: {
    outDir: 'dist',
    assetsDir: 'assets',
    emptyOutDir: true,
  },
  server: {
    port: 5173,
    proxy: {
      // 开发时 API 代理到组长端
      '/v1': 'http://127.0.0.1:39091',
      '/auth': 'http://127.0.0.1:39091',
      '/api': 'http://127.0.0.1:39091',
    },
  },
});