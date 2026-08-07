import { fileURLToPath, URL } from 'node:url'
import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [vue(), tailwindcss()],

  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },

  // Tauri's CLI shows its own build output; letting Vite clear the screen hides it.
  clearScreen: false,

  server: {
    // Must match `devUrl` in tauri.conf.json. strictPort means we fail loudly on a
    // port clash rather than silently starting somewhere Tauri won't look.
    port: 1420,
    strictPort: true,
    watch: {
      // Rust changes are handled by the Tauri CLI; watching them here would trigger
      // pointless frontend reloads on every cargo build.
      ignored: ['**/src-tauri/**'],
    },
  },

  build: {
    // WebView2 tracks Edge, so a modern baseline is safe and produces smaller output.
    target: 'chrome110',
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // Boolean, not 'esbuild': Vite 8 minifies with oxc, and naming esbuild explicitly
    // pulls in a package that is no longer bundled.
    minify: !process.env.TAURI_ENV_DEBUG,
  },

  test: {
    environment: 'happy-dom',
    globals: true,
    include: ['src/**/*.{test,spec}.ts'],
  },
})
