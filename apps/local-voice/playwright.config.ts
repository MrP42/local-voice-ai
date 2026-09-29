import { defineConfig, devices } from "@playwright/test";

// Eigener Port je Worktree: mit `reuseExistingServer` testet ein zweiter
// Lauf sonst stillschweigend gegen den Vite-Server eines anderen Baums.
// vite.config.ts liest dieselbe Variable; ohne sie bleibt es bei 1420.
const port = Number(process.env.LV_DEV_PORT) || 1420;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: "html",
  use: {
    baseURL: `http://localhost:${port}`,
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
    url: `http://localhost:${port}`,
    reuseExistingServer: !process.env.CI,
    timeout: 30000,
  },
});
