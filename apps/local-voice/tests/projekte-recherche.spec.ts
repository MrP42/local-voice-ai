import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import { installProjekteMock } from "./projekteMock";
import { calls, openRecordings, pickMeeting } from "./recLayoutMock";

// ADR-0026 PR-7: Suite-Projekt eines Besprechungsordners mit Projektordner (nur dieses Projekt),
// Recherche (Modus Schnell, Quelle Web) und Freigabe-Dialog. Die Regeln des Backends (Default-Deny,
// Halt, Freigabe, Audit ohne Inhalte) pruefen die Rust-Tests; hier die Oberflaeche gegen die Attrappe.
// Szene der Bilder: Ordner "Kunde Stadtwerke" = Suite-Projekt "Kunde Stadtwerke" (PERSONAL).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../docs/vorschau/projekte-recherche",
);
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const DUNKEL = process.env.LVA_THEME === "dark";
/** Messwerte als Beleg (LVA_BELEGE=<Ordner>): maschinell aus dem Lauf, nicht von Hand. */
const beleg = (name: string, daten: unknown) => {
  const dir = process.env.LVA_BELEGE;
  if (!dir) return;
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(
    path.join(dir, name),
    JSON.stringify(daten, null, 2) + "\n",
    "utf-8",
  );
};
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  // Die Meldung "Die Recherche ist fertig" soll das Bild nicht verdecken.
  await page
    .getByText("Die Recherche ist fertig")
    .waitFor({ state: "hidden", timeout: 15_000 })
    .catch(() => {});
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(
      SHOT_DIR,
      `${name}${DUNKEL ? "-dunkel" : ""}-${page.viewportSize()?.width}.png`,
    ),
    animations: "disabled",
  });
};

const FRAGE = "Wie werden Netzentgelte für die Lastgangmessung berechnet?";

const setup = async (
  page: Page,
  width = 1440,
  height = 900,
  vorher?: (page: Page) => Promise<void>,
) => {
  await installLiveMock(page);
  await installProjekteMock(page);
  if (DUNKEL)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  if (vorher) await vorher(page);
  await openRecordings(page, width, height);
};

const oeffneSuite = async (page: Page, folderId: string) => {
  const row = page.locator(`[data-project-id="${folderId}"]`);
  if (!(await row.isVisible())) {
    // Mittel: Leiste mit Knopf; schmal: Knopf im Seitenkopf.
    const rail = page.getByTestId("sessions-expand");
    await (
      (await rail.count()) > 0 ? rail : page.getByTestId("sessions-open")
    ).click();
  }
  // Erst den Ordner wählen: der markierte Ordner ist der, dessen Suite-Projekt die Seite zeigt.
  await row.click();
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Suite-Projekt …" }).click();
  await expect(page.getByTestId("suite-projekt")).toBeVisible();
};

const frageStellen = async (page: Page, frage: string) => {
  const box = page.getByTestId("suite-recherche").locator("textarea, input");
  await box.first().fill(frage);
  await page
    .getByTestId("suite-recherche")
    .getByRole("button", { name: "Recherchieren" })
    .click();
};

const bus = (page: Page, art: string, detail?: string) =>
  page.evaluate(
    ([a, d]) =>
      window.dispatchEvent(
        new CustomEvent("lva:projekte", { detail: { art: a, detail: d } }),
      ),
    [art, detail] as const,
  );

const suite = (page: Page, key: string, value: unknown) =>
  page.evaluate(([k, v]) => ((window as any).__suite[k as string] = v), [
    key,
    value,
  ] as const);

for (const [width, height] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Projektordner, Halt, Freigabe und Ergebnis (${width} px)`, async ({
    page,
  }) => {
    await setup(page, width, height);
    await oeffneSuite(page, "f3");

    const seite = page.getByTestId("suite-projekt");
    await expect(
      seite.getByText("Kundentermin Stadtwerke Kaiserslautern – Lastgang"),
    ).toBeVisible();
    await expect(seite.getByText("Leitfaden Netzentgelte")).toBeVisible();
    // Der markierte Ordner links ist der gezeigte (nur breit sichtbar).
    await expect(page.locator('[data-project-id="f3"]')).toHaveAttribute(
      "aria-current",
      "true",
    );
    // Nur dieses Projekt, Modus und Quelle fest: keine Auswahl, kein "Gründlich"/"Tief".
    await expect(seite.getByText("Jour fixe Vertrieb KW 40")).toHaveCount(0);
    await expect(seite.getByText("Gründlich")).toHaveCount(0);
    await expect(seite.getByText("Tief", { exact: true })).toHaveCount(0);
    await expect(seite.getByText("Projektquellen")).toHaveCount(0);
    await shoot(page, "besprechungsprojekt-projektordner");

    await frageStellen(page, FRAGE);
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(FRAGE);
    await expect(dialog).toContainText("personenbezogen");
    await expect(dialog).toContainText("noch 15 Minuten");
    await expect(
      dialog.getByText(
        "Netzentgelte Lastgangmessung Stadtwerke Kaiserslautern",
      ),
    ).toBeVisible();
    // "Nur im Projekt suchen" gibt es nicht (kein Knopf ohne Wirkung).
    await expect(
      dialog.getByRole("button", { name: "Nur im Projekt suchen" }),
    ).toHaveCount(0);
    await shoot(page, "freigabedialog");

    // Nichts ist vorab gewählt; "Ausgewählte senden" ist erst mit Auswahl wirksam.
    const senden = dialog.getByRole("button", { name: "Ausgewählte senden" });
    await expect(senden).toBeDisabled();
    await dialog.getByRole("checkbox").nth(0).check();
    await senden.click();
    const e = await calls(page, "recherche_freigabe_entscheiden");
    expect(e).toHaveLength(1);
    expect(e[0].args.aktion).toBe("senden");
    expect(e[0].args.auswahl).toEqual(["q1"]);
    await expect(dialog).toBeHidden();

    await expect(seite.getByText("1 Quelle").first()).toBeVisible();
    await expect(
      seite.getByText("Netzentgelte Strom - Bundesnetzagentur"),
    ).toBeVisible();
    await expect(seite.getByText("1 Quellen")).toHaveCount(0);
    await shoot(page, "recherche-ergebnis");
    // Das Ergebnis liegt im Projektordner; keine zweite Seitenleiste.
    await expect(seite.getByText(FRAGE).first()).toBeVisible();
    const breiter = await page.evaluate(
      () =>
        document.documentElement.scrollWidth >
        document.documentElement.clientWidth,
    );
    expect(breiter).toBe(false);

    // Löschen: Dokument und Eintrag verschwinden.
    await seite.getByTestId("suite-recherche-loeschen").click();
    const l = await calls(page, "recherche_loeschen");
    expect(l).toHaveLength(1);
    await expect(
      seite.getByText("Netzentgelte Strom - Bundesnetzagentur"),
    ).toHaveCount(0);
  });
}

test("Abbrechen sendet nichts und meldet es", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  const e = await calls(page, "recherche_freigabe_entscheiden");
  expect(e[0].args.aktion).toBe("abbrechen");
  expect(e[0].args.auswahl).toEqual([]);
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("suite-recherche")).toContainText(
    "Es wurde nichts gesendet.",
  );
  await expect(
    page.getByTestId("suite-projekt").getByText("1 Quelle"),
  ).toHaveCount(0);
});

test("Ohne Web-Freigabe ist die Recherche gesperrt", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f1");
  await expect(
    page.getByTestId("suite-recherche").getByText("nicht freigegeben"),
  ).toBeVisible();
  await expect(page.getByTestId("suite-web")).not.toBeChecked();
});

test("Doppelklick auf Senden entscheidet nur einmal", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(page, "entscheidenMs", 600);
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog
    .getByRole("button", { name: "Ausgewählte senden" })
    .dblclick({ delay: 20 });
  await expect(dialog).toBeHidden({ timeout: 10_000 });
  expect(await calls(page, "recherche_freigabe_entscheiden")).toHaveLength(1);
});

test("Lesefehler beim Prüfen lässt den Dialog stehen und meldet einmal", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await suite(page, "failFreigaben", true);
  await bus(page, "freigabe");
  await bus(page, "freigabe");
  await expect(
    page.getByText("Offene Freigaben konnten nicht gelesen werden."),
  ).toHaveCount(1);
  await expect(dialog).toBeVisible();
});

test("Während einer Entscheidung wird nicht gelesen", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(page, "entscheidenMs", 1200);
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  const vorher = (await calls(page, "recherche_freigaben")).length;
  await bus(page, "freigabe");
  await bus(page, "freigabe");
  await page.waitForTimeout(300);
  expect((await calls(page, "recherche_freigaben")).length).toBe(vorher);
  await expect(dialog).toBeHidden({ timeout: 10_000 });
});

test("Ein Fehler der Entscheidung gilt nur für diesen Halt", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  await suite(
    page,
    "entscheidenFehler",
    "recherche.invalid: Auswahl passt nicht",
  );
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  await expect(dialog.getByRole("alert")).toContainText("Auswahl");
  // Zweiter Halt: kein alter Fehler im Dialog.
  await suite(page, "entscheidenFehler", null);
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(dialog).toBeHidden();
  await frageStellen(page, "Wie hoch war der Leistungspreis 2025?");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
});

test("Fremde Recherchen erscheinen nicht im falschen Kontext", async ({
  page,
}) => {
  await setup(page);
  await oeffneSuite(page, "f3");
  // Eine Recherche aus einem anderen Kontext meldet sich fertig: diese Seite wartet nicht darauf.
  await bus(page, "recherche", "rc-fremd");
  await page.waitForTimeout(300);
  expect(await calls(page, "recherche_lesen")).toHaveLength(0);
  await expect(page.getByTestId("suite-recherche")).toContainText(
    "Noch keine Recherche",
  );
});

for (const [breite, hoehe] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Chat-Spalte: Projekt wählen, Eingabe bleibt bis zum angenommenen Start (${breite} px)`, async ({
    page,
  }) => {
    await setup(page, breite, hoehe);
    // m2 liegt in zwei Ordnern mit je einem Suite-Projekt: die Person wählt das Projekt.
    await pickMeeting(page, "m2");
    await page
      .getByRole(breite < 620 ? "tab" : "button", {
        name: "Fragen",
        exact: true,
      })
      .click();
    const panel = page.getByTestId("chat-panel");
    const knopf = panel.getByTestId("chat-recherche");
    await expect(knopf).toBeDisabled();
    const eingabe = panel.locator("textarea");
    await eingabe.fill(FRAGE);
    await knopf.click();
    // Vor der Wahl ist noch nichts gestartet, die Frage bleibt stehen.
    await expect(eingabe).toHaveValue(FRAGE);
    await panel.getByRole("button", { name: "Kunde Stadtwerke" }).click();
    const start = await calls(page, "recherche_starten");
    expect(start).toHaveLength(1);
    expect(start[0].args.suiteProjekt).toBe("sp2");
    expect(start[0].args.besprechung).toBe("m2");
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText(FRAGE);
    await expect(eingabe).toHaveValue("");
    await shoot(page, "chat-recherche-gestartet");
    // Nach der Freigabe gehört genau dieses Ergebnis in die Chat-Spalte.
    await dialog.getByRole("checkbox").nth(0).check();
    await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
    await expect(panel.getByTestId("recherche-karte")).toContainText(FRAGE);
    await expect(panel.getByTestId("recherche-abgelegt")).toContainText(
      "Kunde Stadtwerke",
    );
    await shoot(page, "chat-recherche-ergebnis");
  });
}

test("Chat-Spalte: abgebrochener Halt zeigt kein Ergebnis", async ({
  page,
}) => {
  await setup(page);
  await pickMeeting(page, "m2");
  await page.getByRole("button", { name: "Fragen", exact: true }).click();
  const panel = page.getByTestId("chat-panel");
  await panel.locator("textarea").fill(FRAGE);
  await panel.getByTestId("chat-recherche").click();
  await panel.getByRole("button", { name: "Kunde Stadtwerke" }).click();
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("button", { name: "Abbrechen" }).click();
  await expect(panel.getByTestId("chat-notice")).toContainText(
    "Es wurde nichts gesendet.",
  );
  await expect(panel.getByTestId("recherche-karte")).toHaveCount(0);
});

test("Barrierefreiheit der Projektseite", async ({ page }) => {
  await setup(page);
  await oeffneSuite(page, "f2");
  const r = await new AxeBuilder({ page })
    .include('[data-testid="suite-projekt"]')
    .analyze();
  expect(r.violations.filter((v) => v.impact === "critical")).toEqual([]);
});

// --- Schmale Ansicht (390 px): Projektseite im Seitenfluss, Touchziele, Kontrast -------------------

const scrollContainerUm = (page: Page, selector: string) =>
  page.evaluate((sel) => {
    const el = document.querySelector(sel);
    const gefunden: string[] = [];
    for (let n = el?.parentElement ?? null; n; n = n.parentElement) {
      const cs = getComputedStyle(n);
      const scrollt = /(auto|scroll)/.test(cs.overflowY);
      if (scrollt && n.scrollHeight > n.clientHeight + 1)
        gefunden.push(
          `${n.tagName}[${n.getAttribute("data-testid") ?? n.className}]`,
        );
    }
    return gefunden;
  }, selector);

test("390 px: Projektseite im Seitenfluss mit einer Scrollleiste, Reiterblock einklappbar", async ({
  page,
}) => {
  await setup(page, 390, 844);
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  const seite = page.getByTestId("suite-projekt");
  await expect(seite.getByText("1 Quelle").first()).toBeVisible();

  // Genau ein scrollender Vorfahre um das Ergebnis: der Seitenfluss.
  const ergebnis = '[data-testid="suite-projekt"] [aria-live="polite"]';
  const umgebung = await scrollContainerUm(page, ergebnis);
  expect(umgebung).toHaveLength(1);
  expect(umgebung[0]).toContain("rec-flow");

  // Das Fenster dieses Flusses ist fast so hoch wie die Seite (kein kurzes Teilfenster).
  const fluss = page.getByTestId("rec-flow");
  const h = (await fluss.boundingBox())!.height;
  expect(h).toBeGreaterThan(500);

  // Weiterfragen und letzte Quelle sind per Scroll erreichbar.
  const letzte = seite.getByText("Netzentgelte Strom - Bundesnetzagentur");
  await letzte.scrollIntoViewIfNeeded();
  const b = (await letzte.boundingBox())!;
  expect(b.y + b.height).toBeLessThanOrEqual(844);
  beleg("messwerte-fluss.json", {
    viewport: "390x844",
    scrollcontainerUmErgebnis: umgebung,
    anzahlScrollcontainer: umgebung.length,
    flussFensterHoehePx: Math.round(h),
    letzteQuelle: {
      text: "Netzentgelte Strom - Bundesnetzagentur",
      yPx: Math.round(b.y),
      hoehePx: Math.round(b.height),
      imSichtbarenBereich: b.y + b.height <= 844,
    },
    reiterblockUnterSeite: true,
  });
  await expect(
    seite.getByRole("button", {
      name: "Wie hoch war der Leistungspreis 2025?",
    }),
  ).toBeAttached();
  await expect(page.getByText("Die Recherche ist fertig")).toBeHidden({
    timeout: 15_000,
  });
  await shoot(page, "recherche-ergebnis-fluss");

  // Der Reiterblock steht unter der Seite und lässt sich einklappen.
  const reiter = page.getByRole("tab", { name: "Notizen", exact: true });
  const toggle = page.getByTestId("rec-lower-toggle");
  await toggle.scrollIntoViewIfNeeded();
  await expect(reiter).toBeVisible();
  await shoot(page, "recherche-reiterblock");
  const yReiter = (await reiter.boundingBox())!.y;
  const yErgebnis = (await seite.boundingBox())!.y;
  expect(yReiter).toBeGreaterThan(yErgebnis);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(reiter).toBeHidden();
  await toggle.click();
  await expect(reiter).toBeVisible();
});

test("390 px: Touchziele mindestens 44 px (Seite, Aufnahmezeile, Reiter, Dialog)", async ({
  page,
}) => {
  await setup(page, 390, 844);
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important}",
  });
  await oeffneSuite(page, "f3");
  const gemessen: Record<string, unknown[]> = {};
  const zuKlein = async (selector: string, gruppe = selector) => {
    const alle = await messen(selector);
    gemessen[gruppe] = alle;
    return alle.filter((x) => x.w > 0 && (x.h < 44 || x.w < 44));
  };
  const messen = async (selector: string) =>
    page.evaluate((sel) => {
      return [...document.querySelectorAll<HTMLElement>(sel)]
        .filter((e) => e.offsetParent !== null)
        .map((e) => {
          const r = e.getBoundingClientRect();
          return {
            name: (e.getAttribute("aria-label") ?? e.textContent ?? e.tagName)
              .trim()
              .slice(0, 40),
            w: Math.round(r.width),
            h: Math.round(r.height),
          };
        })
        .filter((x) => x.w > 0);
    }, selector);

  expect(
    await zuKlein(
      [
        '[data-testid="sessions-open"]',
        '[data-testid="rec-strip"] button',
        '[data-testid="rec-lower"] [role="tab"]',
        '[data-testid="rec-lower-toggle"]',
        '[data-testid="suite-projekt"] button:not(.wai-recherche__fn)',
      ].join(","),
    ),
  ).toEqual([]);

  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  // Das Einblenden des Dialogs skaliert kurz: erst danach messen.
  await page.waitForTimeout(800);
  expect(
    await zuKlein(
      '[role="alertdialog"] button, [role="alertdialog"] .wai-freigabe__liste label',
    ),
  ).toEqual([]);
  // Die ganze Zeile (Beschriftung) schaltet das Kästchen.
  const zeile = dialog.locator("label").first();
  await zeile.getByText("Netzentgelte Lastgangmessung Berechnung").click();
  await expect(dialog.getByRole("checkbox").first()).toBeChecked();
  const box = (await zeile.boundingBox())!;
  beleg("messwerte-touch.json", {
    viewport: "390x844",
    grenzePx: 44,
    gruppen: gemessen,
    checkboxZeilen: await messen(
      '[role="alertdialog"] .wai-freigabe__liste label',
    ),
  });
  expect(box.height).toBeGreaterThanOrEqual(44);
  expect(box.width).toBeGreaterThan(250);
});

for (const theme of ["light", "dark"] as const) {
  test(`390 px: Projektseite ohne serious/critical, Suchplatzhalter ≥ 4,5:1 (${theme})`, async ({
    page,
  }) => {
    await setup(page, 390, 844, async (p) => {
      await p.addInitScript((value) => {
        (window as any).__settings.theme = value;
      }, theme);
    });
    await oeffneSuite(page, "f3");
    await page.addStyleTag({
      content:
        "*,*::before,*::after{transition:none!important;animation:none!important}",
    });
    // Zwei Suchfelder der Seite: "In … suchen" (Liste, in der Schublade) und "Im Projekt suchen".
    await expect(page.getByPlaceholder("Im Projekt suchen")).toBeVisible();
    await page.waitForTimeout(300);
    const r = await new AxeBuilder({ page })
      .include('[data-testid="suite-projekt"]')
      .exclude("[data-tabster-dummy]")
      .analyze();
    const zusammen = (a: typeof r) => ({
      violations: a.violations,
      passesAnzahl: a.passes.length,
      incompleteAnzahl: a.incomplete.length,
      regelnGeprueft: a.passes
        .map((x) => x.id)
        .concat(a.violations.map((x) => x.id))
        .sort(),
      regelsatz: a.testEngine,
    });
    beleg(`axe-390-${theme === "light" ? "hell" : "dunkel"}.json`, {
      viewport: "390x844",
      theme,
      projektseite: {
        bereich: '[data-testid="suite-projekt"]',
        ...zusammen(r),
      },
    });
    expect(
      r.violations
        .filter((v) => v.impact === "critical" || v.impact === "serious")
        .map(
          (v) =>
            `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`,
        ),
    ).toEqual([]);

    // Schublade mit der Ordnerliste: Suchplatzhalter dort ebenfalls.
    await page.getByTestId("sessions-open").click();
    await expect(page.getByTestId("rec-sessions")).toBeVisible();
    const s2 = await new AxeBuilder({ page })
      .include('[data-testid="rec-sessions"]')
      .exclude("[data-tabster-dummy]")
      .withRules(["color-contrast"])
      .analyze();
    {
      const dir = process.env.LVA_BELEGE;
      const datei = path.join(
        dir ?? "",
        `axe-390-${theme === "light" ? "hell" : "dunkel"}.json`,
      );
      if (dir) {
        const alt = JSON.parse(fs.readFileSync(datei, "utf-8"));
        alt.ordnerliste = {
          bereich: '[data-testid="rec-sessions"]',
          regeln: ["color-contrast"],
          violations: s2.violations,
          passesAnzahl: s2.passes.length,
          incompleteAnzahl: s2.incomplete.length,
          regelsatz: s2.testEngine,
        };
        beleg(`axe-390-${theme === "light" ? "hell" : "dunkel"}.json`, alt);
      }
    }
    expect(
      s2.violations
        .filter((v) => v.impact === "critical" || v.impact === "serious")
        .map(
          (v) =>
            `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`,
        ),
    ).toEqual([]);
  });
}

// --- Vorlesen: Klassenregel für die Sprachausgabe ------------------------------------------------

const ergebnisMitTts = async (
  page: Page,
  einstellungen: Record<string, unknown>,
  breite = 1440,
  hoehe = 900,
) => {
  await setup(page, breite, hoehe, async (p) => {
    await p.addInitScript((e) => {
      Object.assign((window as any).__settings, e);
    }, einstellungen);
  });
  await oeffneSuite(page, "f3");
  await frageStellen(page, FRAGE);
  const dialog = page.getByRole("alertdialog");
  await dialog.getByRole("checkbox").nth(0).check();
  await dialog.getByRole("button", { name: "Ausgewählte senden" }).click();
  await expect(page.getByTestId("suite-recherche-loeschen")).toBeVisible();
};

for (const [breite, hoehe] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Vorlesen: bei PERSONAL und Remote-TTS gesperrt, mit Hinweis (${breite} px)`, async ({
    page,
  }) => {
    await ergebnisMitTts(
      page,
      {
        tts_engine: "provider:cloud1",
        tts_providers: { allow_remote: ["cloud1"] },
      },
      breite,
      hoehe,
    );
    const knopf = page.getByTestId("suite-readaloud");
    await expect(knopf).toBeDisabled();
    console.log(
      `TTS-SPERRE Fall=PERSONAL+Remote-TTS breite=${breite} knopfGesperrt=${await knopf.isDisabled()} hinweisSichtbar=${await page.getByText("nur mit lokaler Sprachausgabe").isVisible()}`,
    );
    await expect(
      page.getByText("nur mit lokaler Sprachausgabe", { exact: false }),
    ).toBeVisible();
    await knopf.scrollIntoViewIfNeeded();
    await shoot(page, "vorlesen-gesperrt");
  });
}

test("Vorlesen: bei PERSONAL und lokaler TTS freigegeben", async ({ page }) => {
  await ergebnisMitTts(page, {
    tts_engine: "provider:lokal1",
    tts_providers: { allow_remote: [] },
  });
  await expect(page.getByTestId("suite-readaloud")).toBeEnabled();
  console.log(
    `TTS-SPERRE Fall=PERSONAL+lokale-TTS knopfFrei=${await page.getByTestId("suite-readaloud").isEnabled()} hinweisSichtbar=${await page.getByText("nur mit lokaler Sprachausgabe").isVisible()}`,
  );
  await expect(
    page.getByText("nur mit lokaler Sprachausgabe", { exact: false }),
  ).toHaveCount(0);
});

for (const [breite, hoehe] of [
  [1440, 900],
  [390, 844],
] as const) {
  test(`Sprachgespräch: Recherche über das Plus-Menü (${breite} px)`, async ({
    page,
  }) => {
    await setup(page, breite, hoehe);
    const nav = page.getByRole("button", {
      name: "Sprachgespräch",
      exact: true,
    });
    if (!(await nav.isVisible()))
      await page.getByRole("button", { name: /Menü/ }).first().click();
    await nav.click();
    await page.getByRole("button", { name: "Hinzufügen" }).click();
    await page.getByRole("menuitem", { name: /Recherche/ }).click();
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("combobox").click();
    await page.getByRole("option", { name: "Kunde Stadtwerke" }).click();
    await dialog.getByLabel("Frage").fill(FRAGE);
    await dialog.getByTestId("sprache-recherche-los").click();
    const start = await calls(page, "recherche_starten");
    expect(start).toHaveLength(1);
    expect(start[0].args.suiteProjekt).toBe("sp2");
    const freigabe = page.getByRole("alertdialog");
    await expect(freigabe).toContainText(FRAGE);
    await shoot(page, "sprache-recherche-gestartet");
    await freigabe.getByRole("checkbox").nth(0).check();
    await freigabe.getByRole("button", { name: "Ausgewählte senden" }).click();
    await expect(freigabe).toBeHidden();
    await expect(
      page.getByTestId("sprache-recherche").getByTestId("recherche-karte"),
    ).toContainText(FRAGE);
    await expect(
      page.getByTestId("sprache-recherche").getByTestId("recherche-abgelegt"),
    ).toContainText("Kunde Stadtwerke");
    await shoot(page, "sprache-recherche-ergebnis");
  });
}
