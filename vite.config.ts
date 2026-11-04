import { defineConfig, UserConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath, URL } from 'node:url';

const host = process.env.TAURI_DEV_HOST;

// E2E mocks (and the `?window=` hook) exist only on the dev server: no build can ship them.
const isE2E = process.env.E2E === 'true';
const isDev = process.env.DEV === 'true';

export default defineConfig(({ command }): UserConfig => ({
  plugins: [
    vue(),
    tailwindcss(),
  ],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  define: {
    __DEV__: isDev,
    __E2E__: isE2E && command === 'serve',
    __APP_VERSION__: JSON.stringify(process.env.npm_package_version),
  },
  clearScreen: false,
  build: {
    chunkSizeWarningLimit: 1000,
  },
  server: {
    // Tauri dev uses 1420; e2e runs its own mocked server on another port (playwright.config.ts).
    port: Number(process.env.VITE_PORT) || 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: 'ws',
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**', '!**/src-tauri/tauri.conf.json', '**/coverage/**', '**/manifest/**', '**/test-results/**', '**/.claude/**'],
    },
  },
}));
