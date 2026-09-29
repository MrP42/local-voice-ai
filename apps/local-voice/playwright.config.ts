import { defineConfig, devices } from "@playwright/test";
import { createHash } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Port je Checkout: LV_DEV_PORT aus der Umgebung, sonst ein stabiler, gerader
// Port aus dem Hash des absoluten Pfads (Vite belegt zusaetzlich Port+1 fuer
// HMR). So laufen Testlaeufe in mehreren Worktrees nicht gegen denselben
// Vite-Server. `tauri dev` bleibt bei 1420 (devUrl in tauri.conf.json).
const pathHash = createHash("sha1")
  .update(resolve(dirname(fileURLToPath(import.meta.url))).toLowerCase())
  .digest()
  .readUInt32BE(0);
const devPort =
  Number(process.env.LV_DEV_PORT) || 21000 + (pathHash % 4000) * 2;
const baseURL = `http://localhost:${devPort}`;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL,
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],
  webServer: {
    // Ueber den Paketmanager des Projekts, nicht ueber bunx: bun ist auf
    // den Windows-Arbeitsplaetzen nicht installiert, und der Testlauf
    // scheiterte dort schon am Start des Servers statt an einem Befund.
    command: "pnpm exec vite dev",
    url: baseURL,
    env: { LV_DEV_PORT: String(devPort) },
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
  },
});
