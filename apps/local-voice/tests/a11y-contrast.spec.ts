import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { installRecMock, openRecordings, pickMeeting } from "./recLayoutMock";

// Farbkontrast app-weit (Goal Issues-Abschluss, G2e, Befund B4 aus #66):
// axe-Regel `color-contrast` (WCAG 2.1 AA: 4,5:1 normaler Text, 3:1 grosser
// Text) auf jeder Hauptseite, im hellen UND im dunklen Erscheinungsbild.
// Die Tokenwerte stehen in src/styles/theme.css (--*-color-text-muted).

const PAGES = [
  { name: "Start", nav: "Start" },
  { name: "Verlauf", nav: "Verlauf" },
  { name: "Aufnahmen", nav: "Aufnahmen" },
  { name: "Vorlesen", nav: "Vorlesen" },
  { name: "Modelle", nav: "Modelle" },
  { name: "Einstellungen", nav: "Einstellungen" },
] as const;

const THEMES = ["light", "dark"] as const;

/** Befunde der Regel color-contrast (serious/critical), lesbar zusammengefasst. */
const contrastFindings = async (page: Page): Promise<string[]> => {
  const result = await new AxeBuilder({ page })
    .withRules(["color-contrast"])
    .analyze();
  return result.violations
    .filter((v) => v.impact === "critical" || v.impact === "serious")
    .flatMap((v) =>
      v.nodes.map((n) => {
        const data = (n.any[0]?.data ?? {}) as {
          fgColor?: string;
          bgColor?: string;
          contrastRatio?: number;
        };
        return `${n.target.join(" ")} ${data.fgColor}/${data.bgColor} ${data.contrastRatio}`;
      }),
    );
};

const goTo = async (page: Page, nav: string) => {
  const button = page.getByRole("button", { name: nav, exact: true });
  if (!(await button.isVisible())) {
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await button.click();
};

for (const theme of THEMES) {
  test.describe(`axe color-contrast - ${theme}`, () => {
    test.beforeEach(async ({ page }) => {
      await installRecMock(page);
      // Nach der Attrappe laufen lassen: Erscheinungsbild der Einstellungen.
      await page.addInitScript((value) => {
        (window as any).__settings.theme = value;
      }, theme);
    });

    for (const target of PAGES) {
      test(`Seite ${target.name} ohne serious/critical`, async ({ page }) => {
        await page.setViewportSize({ width: 1366, height: 768 });
        await page.goto("/");
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await goTo(page, target.nav);
        if (target.name === "Aufnahmen") {
          await page.getByTestId("rec-content").waitFor();
          await pickMeeting(page, "m2");
          await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
        }
        // Uebergaenge (Einblenden) wuerden axe mit halbtransparenten Farben messen.
        await page.addStyleTag({
          content:
            "*,*::before,*::after{transition:none!important;animation:none!important}",
        });
        await page.waitForTimeout(300);
        expect(await contrastFindings(page)).toEqual([]);
      });
    }

    test("Aufnahmen ohne Auswahl und Einstellungen: jeder Reiter", async ({
      page,
    }) => {
      await openRecordings(page, 1366, 768);
      expect(await contrastFindings(page), "Aufnahmen ohne Auswahl").toEqual(
        [],
      );
      await goTo(page, "Einstellungen");
      const tabs = page.getByRole("tab");
      const n = await tabs.count();
      for (let i = 0; i < n; i++) {
        await tabs.nth(i).click();
        await page.waitForTimeout(150);
        expect(
          await contrastFindings(page),
          `Einstellungen, Reiter ${i}`,
        ).toEqual([]);
      }
    });
  });
}

// Alle Schriftklassen, die App.css auf Token abbildet (gedaempft, Markengelb,
// Statusfarben), auf der Seitenflaeche und auf mid-gray/20, auch dort, wo die
// Attrappe sie nicht zeigt (Fehler-, Warn- und Erfolgsmeldungen, Dialoge).
const MAPPED_TEXT_CLASSES = [
  "text-text/60",
  "text-text/50",
  "text-text/45",
  "text-text/40",
  "text-mid-gray",
  "text-mid-gray/70",
  "text-logo-primary",
  ...["300", "400", "500", "600", "700"].flatMap((n) => [
    `text-red-${n}`,
    `text-amber-${n}`,
    `text-yellow-${n}`,
  ]),
  ...["400", "500", "600", "700"].map((n) => `text-green-${n}`),
  "text-orange-400",
  "text-orange-500",
  "text-emerald-400",
  "text-emerald-600",
  "text-emerald-700",
  "text-sky-400",
  "text-sky-600",
  "text-blue-400",
  "text-blue-500",
];

for (const theme of THEMES) {
  test(`axe color-contrast - ${theme}: alle abgebildeten Schriftklassen`, async ({
    page,
  }) => {
    await installRecMock(page);
    await page.addInitScript((value) => {
      (window as any).__settings.theme = value;
    }, theme);
    await page.setViewportSize({ width: 1366, height: 768 });
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.evaluate((classes) => {
      const host = document.createElement("div");
      host.id = "contrast-probe";
      host.style.cssText =
        "position:fixed;inset:0;z-index:9999;overflow:auto;padding:8px;background:var(--color-background)";
      for (const surface of ["", "bg-mid-gray/20"]) {
        const row = document.createElement("div");
        row.className = `${surface} p-2 flex flex-wrap gap-2`;
        for (const cls of classes) {
          const span = document.createElement("span");
          span.className = `${cls} text-sm`;
          span.textContent = `Beispieltext ${cls}`;
          row.appendChild(span);
        }
        host.appendChild(row);
      }
      document.body.appendChild(host);
    }, MAPPED_TEXT_CLASSES);
    const result = await new AxeBuilder({ page })
      .include("#contrast-probe")
      .withRules(["color-contrast"])
      .analyze();
    expect(
      result.violations.flatMap((v) =>
        v.nodes.map((n) => {
          const d = (n.any[0]?.data ?? {}) as Record<string, unknown>;
          return `${n.html.slice(0, 60)} ${d.fgColor}/${d.bgColor} ${d.contrastRatio}`;
        }),
      ),
    ).toEqual([]);
  });
}
