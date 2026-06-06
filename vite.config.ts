import { defineConfig } from "vite";

// Matches the existing src-tauri/tauri.conf.json:
//   devUrl: "http://localhost:1420"   frontendDist: "../dist"
// Add to tauri.conf.json "build" so `tauri dev` / `tauri build` drive Vite:
//   "beforeDevCommand": "npm run dev", "beforeBuildCommand": "npm run build"
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    target: "es2022",
    sourcemap: true,
  },
});
