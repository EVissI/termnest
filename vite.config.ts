import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Settings follow the Tauri recommendations: fixed dev port, no screen clearing,
// do not watch the Rust sources.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
  build: {
    target: 'chrome105',
    sourcemap: false,
    // Loaded from local disk by the webview, chunk size does not matter here.
    chunkSizeWarningLimit: 1024,
  },
});
