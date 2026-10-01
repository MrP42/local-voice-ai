import { defineConfig, devices } from "@playwright/test";
import { createHash } from "node:crypto";
import { cpus } from "node:os";
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

// Worker: ein Viertel der Kerne, hoechstens 8. Der Standard (die Haelfte der Kerne,
// hier 16) startet ebenso viele Chromium-Instanzen gegen EINEN Vite-Server; auf
// einem Arbeitsplatz, der nebenher baut, bekommt dann keine Instanz genug CPU, und
// Klicks warten ueber die Frist hinaus auf das naechste Bild ("waiting for element
// to be visible, enabled and stable"). Die Laufzeit steigt dafuer nur maessig.
const localWorkers = Math.max(2, Math.min(8, Math.floor(cpus().length / 4)));

export default defineConfig({
  testDir: "./tests",
  globalSetup: "./tests/global-setup.ts",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : localWorkers,
  // Fristen sind Fehlergrenzen, keine Messwerte: ein gesunder Test braucht
  // Sekunden, ein ausgelasteter Rechner das Mehrfache.
  timeout: 60_000,
  expect: { timeout: 10_000 },
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
