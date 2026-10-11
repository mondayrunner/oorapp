import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";

// Tauri serves the dev server on a fixed port and reads the build from dist/.
export default defineConfig({
  root: "ui",
  plugins: [vue(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**", "**/target/**"] } },
  build: { outDir: "../dist", emptyOutDir: true, target: "safari16" },
});
