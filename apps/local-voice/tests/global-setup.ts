import { chromium, type FullConfig } from "@playwright/test";

// Waermt den Vite-Entwicklungsserver an, bevor die ersten Tests laufen.
//
// Vite uebersetzt jedes Modul erst, wenn es zum ersten Mal angefragt wird. Ohne
// Anwaermen holen alle Worker im selben Augenblick dieselben Hunderte Module und
// warten gemeinsam auf den Server: auf einem Rechner, der nebenher baut, reicht das
// fuer ein `page.goto` ueber die 30 s des Testlimits, ohne dass an der Seite etwas
// falsch waere. Nach diesem einen Besuch liegen die Module im Zwischenspeicher des
// Servers, und jeder Test ladet sie in Millisekunden.
//
// Ein Fehler hier ist kein Testfehler: ohne Anwaermen laufen die Tests wie zuvor.
export default async function globalSetup(config: FullConfig) {
  const baseURL = config.projects[0]?.use.baseURL;
  if (!baseURL) return;
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    // Die Seite meldet ohne Tauri-Bruecke Fehler; das ist hier gleichgueltig, es
    // zaehlt nur, dass alle Module des Einstiegs angefragt werden.
    page.on("pageerror", () => {});
    await page.goto(baseURL, { waitUntil: "networkidle", timeout: 120_000 });
  } catch {
    // siehe oben
  } finally {
    await browser.close();
  }
}
