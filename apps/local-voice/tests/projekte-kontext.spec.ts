import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as fs from "node:fs";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import { installProjekteKontextMock } from "./projekteMock";

// lva#365 P4b: Bereich "Projekte" mit "Als Kontext". Die Regeln des Backends (Vorgaben, Stand,
// Token-Schaetzung) pruefen die Rust-Tests; hier die Oberflaeche gegen die Attrappe
// (tests/projekteMock.ts, Szene "Finanzwissen" wie in docs/vorschau/projekte-p4b).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "docs/vorschau/projekte-p4b/umsetzung",
);

const setup = async (
  page: Page,
  opts: {
    width?: number;
    height?: number;
    dark?: boolean;
    vorher?: (page: Page) => Promise<void>;
    ziel?: string;
  } = {},
) => {
  const {
    width = 1440,
    height = 900,
    dark = false,
    vorher,
    ziel = "Projekte",
  } = opts;
  await installLiveMock(page);
  await installProjekteKontextMock(page);
  if (dark)
    await page.addInitScript(() => {
      (window as any).__settings.theme = "dark";
    });
  if (vorher) await vorher(page);
  await page.setViewportSize({ width, height });
  await page.goto("/");
  await geheZu(page, ziel);
};

const geheZu = async (page: Page, name: string) => {
  const nav = page.getByRole("button", { name, exact: true });
  if (!(await nav.isVisible())) {
    const toggle = page.getByRole("button", { name: /Menü/ }).first();
    if (await toggle.isVisible()) await toggle.click();
  }
  await nav.click();
};

const bereich = (page: Page) => page.getByTestId("projekte-bereich");
const aufrufe = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: any) => x.cmd === c),
    cmd,
  );
const schalter = (page: Page, name: string) =>
  page.getByRole("switch", { name: new RegExp(`^${name}.* als Kontext`) });
const zeile = (page: Page, titel: string) =>
  page.getByTestId("projekt-eintrag").filter({ hasText: titel });
/** Schmal (<= 620 px): erst ein Projekt waehlen, die Liste steht allein. */
const waehleProjekt = async (page: Page, titel: string) => {
  await page
    .getByTestId("projekt-zeile")
    .filter({ hasText: titel })
    .first()
    .click();
};

test.describe("Projekte mit Als Kontext", () => {
  test("Bereich, Gruppen und Zaehler aus einer Quelle", async ({ page }) => {
    await setup(page);
    await expect(bereich(page)).toBeVisible();
    // Navigation: eigener Eintrag "Projekte"
    await expect(
      page.getByRole("button", { name: "Projekte", exact: true }),
    ).toHaveAttribute("aria-current", "page");
    // genau ein h1, und der steht nicht in der Seite selbst
    await expect(page.locator("h1")).toHaveCount(1);
    await expect(bereich(page).locator("h1")).toHaveCount(0);

    await expect(page.getByTestId("gruppe")).toHaveText([
      /^Bücher\s*2$/,
      /^Texte\s*1$/,
      /^Aufnahmen\s*2$/,
      /^Dateien im Projektordner\s*3 Kopien$/,
      /^Recherchen\s*1$/,
    ]);
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
    // Kopf und Liste links: Laenge von projektEintraege (der Zaehler `entryCount` der Attrappe
    // sagt absichtlich 99 und darf nirgends erscheinen).
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "9 Einträge",
    );
    const links = page.getByTestId("projekt-zeile");
    await expect(links.nth(0).getByTestId("projekt-zahl")).toHaveText("9");
    await expect(links.nth(1).getByTestId("projekt-zahl")).toHaveText("23");
    await expect(links.nth(2).getByTestId("projekt-zahl")).toHaveText("17");
    await expect(page.getByText("99", { exact: true })).toHaveCount(0);

    await waehleProjekt(page, "KI-Strategie LRA");
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "23 Einträge",
    );
    await expect(
      page.getByTestId("projekt-zeile").nth(1).getByTestId("projekt-zahl"),
    ).toHaveText("23");

    // Summe, Fenster, "Nicht im Kontext", Erklaerkarte, Karte "KI im Projekt"
    await waehleProjekt(page, "Finanzwissen");
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^15\s000 von 128\s000 Token$/,
    );
    await expect(page.getByTestId("kontext-anzahl")).toContainText(
      "5 Einträge als Kontext",
    );
    await expect(page.getByTestId("kontext-anzahl")).toContainText(
      "Modellfenster GPT-6.1-Terra",
    );
    const fehlt = page.getByTestId("nicht-im-kontext");
    await expect(fehlt).toContainText("1 Buch ohne Kurzbuch");
    await expect(fehlt).toContainText(
      "2 Dateien, deren Format nicht lesbar ist",
    );
    await expect(fehlt).toContainText("1 Eintrag ausgeschaltet");
    await expect(page.getByText("So gilt die Auswahl")).toBeVisible();
    await expect(
      page.getByText(/Die Auswahl gilt auf diesem Gerät/),
    ).toBeVisible();
    await expect(page.getByTestId("ki-im-projekt")).toContainText("Gesperrt");
  });

  test("Eintrag: Token, Hinweis statt Inhalt, Formathinweis mit gesperrtem Schalter", async ({
    page,
  }) => {
    await setup(page);
    await expect(zeile(page, "Rich Dad Poor Dad")).toContainText(
      /~9\s000 Token/,
    );
    const ew = zeile(page, "Der Einfache Weg");
    await expect(ew.getByTestId("kontext-hinweis")).toContainText(
      "Kein Kurzbuch – erstellen oder Volltext wählen.",
    );
    await expect(ew.getByTestId("kontext-hinweis")).toContainText(
      "Nicht im Kontext.",
    );
    const pdf = schalter(page, "Depotauszug-2026.pdf");
    await expect(pdf).toHaveAttribute("aria-disabled", "true");
    await expect(zeile(page, "Depotauszug-2026.pdf")).toContainText(
      "Format nicht als Kontext lesbar",
    );
    await pdf.click({ force: true });
    expect(await aufrufe(page, "projekt_kontext_schalten")).toHaveLength(0);
    // Telefonat Bank startet aus (automatisch zugeordnet)
    await expect(schalter(page, "Telefonat Bank")).toHaveAttribute(
      "aria-checked",
      "false",
    );
    await expect(zeile(page, "Telefonat Bank")).toContainText(/aus · ~1\s100/);
  });

  test("Schalter per Tastatur: Tab und Leertaste rufen projektKontextSchalten", async ({
    page,
  }) => {
    await setup(page);
    const davor = page.getByRole("button", {
      name: "Weitere Aktionen für Beratung Altersvorsorge 05.11.",
    });
    await davor.focus();
    await page.keyboard.press("Tab");
    const bank = schalter(page, "Telefonat Bank");
    await expect(bank).toBeFocused();
    await expect(bank).toHaveAttribute("role", "switch");
    await page.keyboard.press("Space");
    await expect(bank).toHaveAttribute("aria-checked", "true");
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toMatchObject({
      suiteId: "p-fin",
      eintragId: "a2",
      on: true,
      form: null,
      edition: null,
    });
    // Summe wandert mit: 15 000 + 1 100
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^16\s100 von 128\s000 Token$/,
    );
    await expect(page.getByTestId("kontext-anzahl")).toContainText(
      "6 Einträge als Kontext",
    );
    // und wieder aus
    await page.keyboard.press("Space");
    await expect(bank).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^15\s000 von 128\s000 Token$/,
    );
  });

  test("Formwechsel Kurzbuch zu Volltext, Fassung, und Banner zu gross", async ({
    page,
  }) => {
    await setup(page);
    await expect(page.getByTestId("banner-zugross")).toHaveCount(0);
    const form = page
      .getByRole("button", { name: "Form wählen: Kurzbuch" })
      .first();
    await form.click();
    const menue = page.getByRole("menu", { name: "Form und Fassung" });
    await expect(menue).toBeVisible();
    await expect(
      menue.getByRole("menuitemradio", { name: /Kurzbuch/ }),
    ).toHaveAttribute("aria-checked", "true");
    // Fassung je Mitgliedschaft (Buch mit zwei Fassungen)
    await expect(
      menue.getByRole("menuitemradio", { name: "English" }),
    ).toBeVisible();
    await menue.getByRole("menuitemradio", { name: /Volltext/ }).click();
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toMatchObject({
      suiteId: "p-fin",
      eintragId: "rd",
      on: true,
      form: "volltext",
      edition: null,
    });
    await expect(
      page.getByRole("button", { name: "Form wählen: Volltext" }),
    ).toBeVisible();
    // 15 000 - 9 000 + 182 000 = 188 000 > 128 000
    const banner = page.getByTestId("banner-zugross");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(/188\s000 von 128\s000 Token/);
    await expect(banner).toContainText("Es wird nichts still gekürzt.");
    await expect(banner).toContainText(
      "KI-Aufrufe im Projekt sind gesperrt, bis der Kontext ins Modellfenster passt.",
    );
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^188\s000 von 128\s000 Token$/,
    );
    // Fassung waehlen
    await page.getByRole("button", { name: "Form wählen: Volltext" }).click();
    await page
      .getByRole("menu", { name: "Form und Fassung" })
      .getByRole("menuitemradio", { name: "English" })
      .click();
    const zweiter = (await aufrufe(page, "projekt_kontext_schalten"))[1];
    expect(zweiter.args).toMatchObject({
      eintragId: "rd",
      form: null,
      edition: "ed_rd_en",
    });
    // zurueck auf Kurzbuch: Banner weg
    await page.getByRole("button", { name: "Form wählen: Volltext" }).click();
    await page.getByRole("menuitemradio", { name: /Kurzbuch/ }).click();
    await expect(page.getByTestId("banner-zugross")).toHaveCount(0);
  });

  test("Hinweis „Volltext wählen“ schaltet die Form des Buches ohne Kurzbuch", async ({
    page,
  }) => {
    await setup(page);
    await zeile(page, "Der Einfache Weg")
      .getByRole("button", { name: "Volltext wählen" })
      .click();
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls[0].args).toMatchObject({ eintragId: "ew", form: "volltext" });
  });

  test("Reset: Warnbanner, alles aus", async ({ page }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          (window as any).__kontext.reset = true;
        }),
    });
    const banner = page.getByTestId("banner-reset");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(
      "Die Kontextauswahl dieses Geräts war nicht lesbar",
    );
    await expect(banner).toContainText("alle Einträge ausgeschaltet");
    for (const s of await page.getByTestId("kontext-schalter").all())
      await expect(s).toHaveAttribute("aria-checked", "false");
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^0 von 128\s000 Token$/,
    );
    await expect(page.getByTestId("kontext-anzahl")).toContainText(
      "Keine Einträge als Kontext",
    );
  });

  test("Leeres Projekt: Zaehler links und im Kopf sind gleich 0", async ({
    page,
  }) => {
    await setup(page);
    await waehleProjekt(page, "Neue Ideen");
    await expect(page.getByTestId("projekt-leer")).toBeVisible();
    await expect(page.getByTestId("projekt-leer")).toContainText(
      "Neue Ideen ist noch leer",
    );
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "0 Einträge",
    );
    await expect(
      page
        .getByTestId("projekt-zeile")
        .filter({ hasText: "Neue Ideen" })
        .getByTestId("projekt-zahl"),
    ).toHaveText("0");
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^0 von 128\s000 Token$/,
    );
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(0);
  });

  test("Gefundene Datei wird angeboten und erst mit Aufnehmen aufgenommen", async ({
    page,
  }) => {
    await setup(page);
    const banner = page.getByTestId("banner-gefunden");
    await expect(banner).toContainText("Kontoauszug-Okt.txt");
    await expect(banner).toContainText("Sie wurde nicht aufgenommen.");
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
    await banner.getByRole("button", { name: "Aufnehmen" }).click();
    await expect(page.getByTestId("banner-gefunden")).toHaveCount(0);
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(10);
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "10 Einträge",
    );
    await expect(
      page.getByTestId("projekt-zeile").first().getByTestId("projekt-zahl"),
    ).toHaveText("10");
    // aufgenommen heisst nicht eingeschaltet
    await expect(schalter(page, "Kontoauszug-Okt.txt")).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  test("Entfernen: Datei in den Papierkorb, App-Inhalt nur Verknuepfung", async ({
    page,
  }) => {
    await setup(page);
    // Datei: Dialog mit Papierkorb-Hinweis
    await page
      .getByRole("button", {
        name: "Weitere Aktionen für Depotauszug-2026.pdf",
      })
      .click();
    await page
      .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
      .click();
    const dlg = page.getByRole("dialog");
    await expect(dlg).toContainText(
      "„Depotauszug-2026.pdf“ aus dem Projekt nehmen?",
    );
    await expect(dlg).toContainText("Papierkorb des Projekts");
    await expect(dlg).toContainText("Dein Original bleibt unberührt");
    await dlg.getByTestId("entfernen-bestaetigen").click();
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(8);
    const datei = await aufrufe(page, "projekt_datei_entfernen");
    expect(datei).toHaveLength(1);
    expect(datei[0].args).toMatchObject({ suiteId: "p-fin", dateiId: "d2" });
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "8 Einträge",
    );

    // Buch: nur die Verknuepfung
    await page
      .getByRole("button", { name: "Weitere Aktionen für Rich Dad Poor Dad" })
      .click();
    await page
      .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
      .click();
    const dlg2 = page.getByRole("dialog");
    await expect(dlg2).toContainText("Das Buch bleibt in der Bibliothek");
    await expect(dlg2).toContainText(
      "Nur die Verknüpfung zu „Finanzwissen“ entfällt",
    );
    await dlg2.getByRole("button", { name: "Abbrechen" }).first().click();
    expect(await aufrufe(page, "projekt_buch_entfernen")).toHaveLength(0);
    await page
      .getByRole("button", { name: "Weitere Aktionen für Rich Dad Poor Dad" })
      .click();
    await page
      .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
      .click();
    await page.getByTestId("entfernen-bestaetigen").click();
    const buch = await aufrufe(page, "projekt_buch_entfernen");
    expect(buch[0].args).toMatchObject({ suiteId: "p-fin", buchId: "rd" });
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(7);
  });

  test("Hinzufuegen: App-Inhalte verknuepfen und Dateien kopieren", async ({
    page,
  }) => {
    await setup(page);
    await page.getByTestId("hinzufuegen").click();
    const dlg = page.getByRole("dialog");
    await expect(dlg).toContainText("Zu „Finanzwissen“ hinzufügen");
    await expect(dlg.getByRole("tab", { name: "Aus der App" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // was schon im Projekt ist, wird nicht angeboten
    await expect(dlg.getByText("Rich Dad Poor Dad")).toHaveCount(0);
    await dlg.getByRole("checkbox", { name: /The Burning Room/ }).check();
    await dlg
      .getByRole("checkbox", { name: /Rundschreiben Posteingang/ })
      .check();
    await dlg.getByTestId("hinzufuegen-bestaetigen").click();
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(11);
    expect(await aufrufe(page, "projekt_buch_zuordnen")).toHaveLength(1);
    expect(await aufrufe(page, "projekt_seite_zuordnen")).toHaveLength(1);

    // Dateien: Auswahl des Systems, dann kopiert das Backend
    await page.evaluate(() => {
      (window as any).__pick = [
        "/home/test/Kontoauszug.txt",
        "/home/test/Plan.md",
      ];
    });
    await page.getByTestId("hinzufuegen").click();
    await page.getByRole("tab", { name: "Dateien kopieren" }).click();
    await expect(page.getByRole("dialog")).toContainText(
      "Das Original bleibt unverändert",
    );
    await page.getByTestId("dateien-waehlen").click();
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(13);
    const kopie = await aufrufe(page, "projekt_dateien_hinzufuegen");
    expect(kopie[0].args).toMatchObject({
      suiteId: "p-fin",
      paths: ["/home/test/Kontoauszug.txt", "/home/test/Plan.md"],
    });
  });

  test("Gruppe Projekte in Vorlesen und Sprachgespraech, Klick oeffnet die Projektseite", async ({
    page,
  }) => {
    await setup(page, { ziel: "Vorlesen" });
    const gruppe = page.locator('[data-testid="projekte-gruppe"]:visible');
    await expect(gruppe).toBeVisible();
    await expect(gruppe.getByTestId("projekte-gruppe-zeile")).toHaveCount(4);
    await expect(
      gruppe
        .getByTestId("projekte-gruppe-zeile")
        .filter({ hasText: "Finanzwissen" }),
    ).toContainText("9");

    await geheZu(page, "Start");
    const gruppe2 = page.locator('[data-testid="projekte-gruppe"]:visible');
    await expect(gruppe2).toBeVisible();
    await expect(
      gruppe2
        .getByTestId("projekte-gruppe-zeile")
        .filter({ hasText: "KI-Strategie LRA" }),
    ).toContainText("23");
    await gruppe2
      .getByTestId("projekte-gruppe-zeile")
      .filter({ hasText: "KI-Strategie LRA" })
      .click();
    await expect(bereich(page)).toBeVisible();
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "23 Einträge",
    );
    await expect(
      page.getByRole("heading", { name: "KI-Strategie LRA", level: 2 }),
    ).toBeVisible();
  });

  for (const dark of [false, true]) {
    test(`axe ohne Verstoesse inkl. color-contrast (${dark ? "dunkel" : "hell"})`, async ({
      page,
    }) => {
      await setup(page, { dark });
      await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
      await page.addStyleTag({
        content:
          "*,*::before,*::after{transition:none!important;animation:none!important}",
      });
      await page.waitForTimeout(300);
      const pruefen = async (wo: string) => {
        const r = await new AxeBuilder({ page })
          .include('[data-testid="projekte-bereich"]')
          .exclude("[data-tabster-dummy]")
          .withTags([
            "wcag2a",
            "wcag2aa",
            "wcag21a",
            "wcag21aa",
            "best-practice",
          ])
          .analyze();
        const befunde = r.violations.flatMap((v) =>
          v.nodes.map(
            (n) =>
              `${wo}: ${v.id} ${n.target.join(" ")} ${n.failureSummary?.split("\n")[1] ?? ""}`,
          ),
        );
        expect(befunde).toEqual([]);
      };
      await pruefen("Seite");
      // mit Banner zu gross und offenem Formmenue
      await page
        .getByRole("button", { name: "Form wählen: Kurzbuch" })
        .first()
        .click();
      await page.getByRole("menuitemradio", { name: /Volltext/ }).click();
      await expect(page.getByTestId("banner-zugross")).toBeVisible();
      await pruefen("Zu gross");
      await page.getByRole("button", { name: "Form wählen: Volltext" }).click();
      const menue = await new AxeBuilder({ page })
        .include('[role="menu"]')
        .withTags(["wcag2a", "wcag2aa", "wcag21aa"])
        .analyze();
      expect(menue.violations.map((v) => v.id)).toEqual([]);
      await page.keyboard.press("Escape");
      await expect(page.getByRole("menu")).toHaveCount(0);
      // Dialog "Hinzufuegen"
      await page.getByTestId("hinzufuegen").click();
      await expect(page.getByRole("dialog")).toBeVisible();
      const dlg = await new AxeBuilder({ page })
        .include('[role="dialog"]')
        .withRules(["color-contrast"])
        .analyze();
      expect(
        dlg.violations.flatMap((v) =>
          v.nodes.map((n) => `${v.id} ${n.target.join(" ")}`),
        ),
      ).toEqual([]);
    });
  }

  for (const breite of [390, 1440]) {
    test(`keine horizontale Scrollleiste bei ${breite} px, Ziele mindestens 44 px`, async ({
      page,
    }) => {
      await setup(page, { width: breite, height: breite === 390 ? 844 : 900 });
      if (breite === 390) {
        // schmal: zuerst nur die Projektliste, dann die Seite
        await expect(page.getByTestId("projekt-eintrag")).toHaveCount(0);
        await waehleProjekt(page, "Finanzwissen");
      }
      await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
      const mass = await page.evaluate(() => {
        const de = document.documentElement;
        const ueberlauf = Array.from(
          document.querySelectorAll<HTMLElement>(
            ".pb-wrap, .pb, .pb-col, .pb-liste, .pb-ent, .pb-phead, .pb-banner",
          ),
        )
          .filter((e) => e.scrollWidth > e.clientWidth + 1)
          .map((e) => `${e.className} ${e.scrollWidth}>${e.clientWidth}`);
        const klein = Array.from(
          document.querySelectorAll<HTMLElement>(
            ".pb button, .pb [role=switch], .pb-menue button",
          ),
        )
          .filter((e) => e.offsetParent !== null)
          .map((e) => ({
            n: (e.getAttribute("aria-label") ?? e.textContent ?? "")
              .trim()
              .slice(0, 40),
            w: Math.round(e.getBoundingClientRect().width),
            h: Math.round(e.getBoundingClientRect().height),
          }))
          // Hinweis-Links sind Textknoepfe mit 44 px Hoehe; geprueft wird die Hoehe, bei
          // Symbolen auch die Breite.
          .filter((e) => e.h < 44 || (e.n === "" && e.w < 44));
        return {
          seite: de.scrollWidth - de.clientWidth,
          body: document.body.scrollWidth - document.body.clientWidth,
          ueberlauf,
          klein,
        };
      });
      expect(mass.seite).toBeLessThanOrEqual(0);
      expect(mass.body).toBeLessThanOrEqual(0);
      expect(mass.ueberlauf).toEqual([]);
      expect(mass.klein).toEqual([]);
    });
  }

  test("mittel und ultrabreit: Summenleiste unten bzw. drei Spalten", async ({
    page,
  }) => {
    await setup(page, { width: 768, height: 900 });
    await expect(page.getByTestId("summenleiste")).toBeVisible();
    await expect(page.getByTestId("kontext-summe-text")).toBeHidden();
    await page.getByRole("button", { name: "Details" }).click();
    await expect(page.getByTestId("kontext-details")).toContainText(
      "So gilt die Auswahl",
    );
    await page.keyboard.press("Escape");
    await page.setViewportSize({ width: 2560, height: 1300 });
    await expect(page.getByTestId("summenleiste")).toBeHidden();
    await expect(page.getByTestId("kontext-summe-text")).toBeVisible();
  });

  test("Dateiliste defekt: Warnbanner, Neuaufbau mit Bestaetigung", async ({
    page,
  }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          (window as any).__kontext.lage = "defekt";
        }),
    });
    const banner = page.getByTestId("banner-lage");
    await expect(banner).toContainText(
      "Die Dateiliste dieses Projekts fehlt oder ist beschädigt. Dateien im Projektordner werden deshalb nicht angefasst.",
    );
    await page.getByTestId("lage-knopf").click();
    const dlg = page.getByRole("dialog");
    await expect(dlg).toContainText("beiseitegelegt, nicht gelöscht");
    expect(await aufrufe(page, "projekt_dateien_neu_aufbauen")).toHaveLength(0);
    await dlg.getByTestId("neu-aufbauen-bestaetigen").click();
    const calls = await aufrufe(page, "projekt_dateien_neu_aufbauen");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toMatchObject({ suiteId: "p-fin" });
    await expect(page.getByTestId("banner-lage")).toHaveCount(0);
    await expect(page.getByTestId("banner-gefunden")).toBeVisible();
  });

  test("Dateiliste neuer: Hinweis ohne Knopf", async ({ page }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          (window as any).__kontext.lage = "neuer";
        }),
    });
    await expect(page.getByTestId("banner-lage")).toContainText(
      "von einer neueren App-Version",
    );
    await expect(page.getByTestId("lage-knopf")).toHaveCount(0);
  });

  test("Schalten scheitert: Fehler sichtbar, Stand wird neu gelesen", async ({
    page,
  }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          (window as any).__kontext.failSchalten = true;
        }),
    });
    await schalter(page, "Telefonat Bank").click();
    await expect(page.getByRole("alert")).toContainText(
      "Schalten ist nicht möglich",
    );
    await expect(schalter(page, "Telefonat Bank")).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  const kontextSetzen = (page: Page, f: string) =>
    page.evaluate(`(() => { const K = window.__kontext; ${f} })()`);
  const uebersichtAufrufe = async (page: Page) =>
    (await aufrufe(page, "projekt_kontext_uebersicht")).length;

  test("Projektwechsel: späte Antwort von Projekt A landet nicht unter B", async ({
    page,
  }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          const K = (window as any).__kontext;
          K.lage = "defekt";
          for (const c of [
            "projekt_kontext_uebersicht",
            "projekt_dateien_gefunden",
            "projekt_dateien_lage",
          ])
            K.delay[`${c}:p-fin`] = 1200;
        }),
    });
    // A (Finanzwissen) antwortet langsam; vorher wird zu B gewechselt.
    await waehleProjekt(page, "KI-Strategie LRA");
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "23 Einträge",
    );
    const summe = page.getByTestId("kontext-summe-text");
    await expect(summe).not.toHaveText(/^15\s000 von/);
    const text = await summe.textContent();
    await page.waitForTimeout(2000);
    await expect(summe).toHaveText(text ?? "");
    await expect(page.getByTestId("banner-gefunden")).toHaveCount(0);
    await expect(page.getByTestId("banner-lage")).toHaveCount(0);
    // und A zeigt seine eigenen Banner, wenn man zurückgeht
    await waehleProjekt(page, "Finanzwissen");
    await expect(page.getByTestId("banner-gefunden")).toBeVisible();
    await expect(page.getByTestId("banner-lage")).toBeVisible();
  });

  test("Laden während des Schaltens: der Schaltstand bleibt", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(
      page,
      `K.delay["projekt_kontext_uebersicht:p-fin"] = 1200; K.delay["projekt_kontext_schalten"] = 100;`,
    );
    const davor = await uebersichtAufrufe(page);
    // Aufnehmen löst eine neue (langsame) Ladung aus, deren Stand noch "Bank aus" ist.
    await page
      .getByTestId("banner-gefunden")
      .getByRole("button", { name: "Aufnehmen" })
      .click();
    await expect.poll(() => uebersichtAufrufe(page)).toBeGreaterThan(davor);
    const bank = schalter(page, "Telefonat Bank");
    await bank.click();
    await expect(bank).toHaveAttribute("aria-checked", "true");
    await page.waitForTimeout(1800);
    await expect(bank).toHaveAttribute("aria-checked", "true");
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^16\s100 von 128\s000 Token$/,
    );
  });

  test("Aus + Formwechsel: der Eintrag bleibt aus, der Aufruf trägt on=false", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(page, `K.delay["projekt_kontext_schalten"] = 600;`);
    const rd = schalter(page, "Rich Dad Poor Dad");
    await expect(rd).toHaveAttribute("aria-checked", "true");
    await rd.click();
    // Die Anzeige sagt noch "an": der Wechsel darf den Eintrag trotzdem nicht einschalten.
    await page
      .getByRole("button", { name: "Form wählen: Kurzbuch" })
      .first()
      .click();
    await page
      .getByRole("menu", { name: "Form und Fassung" })
      .getByRole("menuitemradio", { name: /Volltext/ })
      .click();
    await expect
      .poll(
        async () => (await aufrufe(page, "projekt_kontext_schalten")).length,
      )
      .toBe(2);
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls[0].args).toMatchObject({
      eintragId: "rd",
      on: false,
      form: null,
    });
    expect(calls[1].args).toMatchObject({
      eintragId: "rd",
      on: false,
      form: "volltext",
    });
    await page.waitForTimeout(1000);
    await expect(rd).toHaveAttribute("aria-checked", "false");
    await expect(
      page.getByRole("button", { name: "Form wählen: Volltext" }),
    ).toBeVisible();
  });

  test("Zwei Eintraege gleichzeitig schalten: beide Aenderungen gelten", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(page, `K.delay["projekt_kontext_schalten"] = 500;`);
    const bank = schalter(page, "Telefonat Bank");
    const beratung = schalter(page, "Beratung Altersvorsorge");
    await expect(beratung).toHaveAttribute("aria-checked", "true");
    await bank.click();
    await beratung.click();
    await expect(bank).toHaveAttribute("aria-checked", "true");
    await expect(beratung).toHaveAttribute("aria-checked", "false");
    await page.waitForTimeout(1200);
    await expect(bank).toHaveAttribute("aria-checked", "true");
    await expect(beratung).toHaveAttribute("aria-checked", "false");
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls).toHaveLength(2);
    // 15 000 + 1 100 - 2 400
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^13\s700 von 128\s000 Token$/,
    );
  });

  test("Fassung ohne zweite Form: das Menü erscheint trotzdem", async ({
    page,
  }) => {
    await setup(page);
    await waehleProjekt(page, "KI-Strategie LRA");
    await page
      .getByRole("button", { name: "Fassung wählen: Alles Idioten!?" })
      .click();
    const menue = page.getByRole("menu", { name: "Form und Fassung" });
    await expect(menue.getByRole("menuitemradio")).toHaveCount(2);
    await menue.getByRole("menuitemradio", { name: "English" }).click();
    const calls = await aufrufe(page, "projekt_kontext_schalten");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toMatchObject({
      eintragId: "ai",
      on: false,
      form: null,
      edition: "ed_ai_en",
    });
  });

  test("Dateien kopieren: während der Kopie nur ein Aufruf, auch bei neu geöffnetem Dialog", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(
      page,
      `window.__pick = ["/home/test/Plan.md"]; K.delay["projekt_dateien_hinzufuegen"] = 1000;`,
    );
    const knopf = page.getByTestId("dateien-waehlen");
    await page.getByTestId("hinzufuegen").click();
    await page.getByRole("tab", { name: "Dateien kopieren" }).click();
    await knopf.click();
    await expect(knopf).toBeDisabled();
    await expect(page.getByTestId("kopier-hinweis")).toHaveText(
      "Dateien werden kopiert …",
    );
    await knopf.click({ force: true });
    // Dialog schließen und wieder öffnen: die Sperre gilt weiter
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByTestId("hinzufuegen").click();
    await page.getByRole("tab", { name: "Dateien kopieren" }).click();
    await expect(knopf).toBeDisabled();
    await knopf.click({ force: true });
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(10, {
      timeout: 5000,
    });
    expect(await aufrufe(page, "projekt_dateien_hinzufuegen")).toHaveLength(1);
    // Der neu geöffnete Dialog bleibt offen; danach ist die Sperre frei.
    await expect(page.getByRole("dialog")).toHaveCount(1);
    await expect(knopf).toBeEnabled();
  });

  test("Entfernen meldet false: nur die Warnung, keine Erfolgsmeldung", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(page, `K.entfernenUnsicher = true;`);
    await page
      .getByRole("button", {
        name: "Weitere Aktionen für Depotauszug-2026.pdf",
      })
      .click();
    await page
      .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
      .click();
    await page.getByTestId("entfernen-bestaetigen").click();
    const toasts = page.locator("[data-sonner-toast]");
    await expect(
      toasts.filter({ hasText: "nicht sicher entfernt" }),
    ).toHaveCount(1);
    await page.waitForTimeout(600);
    await expect(
      toasts.filter({ hasText: "Aus dem Projekt genommen." }),
    ).toHaveCount(0);
  });

  test("Dialog: Ladefehler statt leerer Liste, dann Erneut versuchen", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(page, `K.fail.pages_list = "Seiten nicht lesbar";`);
    await page.getByTestId("hinzufuegen").click();
    const dlg = page.getByRole("dialog");
    await expect(dlg.getByTestId("dialog-ladefehler")).toContainText(
      "Die Liste konnte nicht geladen werden.",
    );
    await expect(dlg.getByTestId("dialog-nichts")).toHaveCount(0);
    await kontextSetzen(page, `delete K.fail.pages_list;`);
    await dlg.getByTestId("dialog-erneut").click();
    await expect(dlg.getByTestId("dialog-ladefehler")).toHaveCount(0);
    await expect(
      dlg.getByRole("checkbox", { name: /The Burning Room/ }),
    ).toBeVisible();
  });

  test("Dialog: mehr als 500 Aufnahmen, Weitere laden", async ({ page }) => {
    await setup(page);
    await kontextSetzen(page, `K.aufnahmen = 620;`);
    await page.getByTestId("hinzufuegen").click();
    const dlg = page.getByRole("dialog");
    const aufnahmen = dlg.getByRole("checkbox", { name: /Aufnahme \d+/ });
    await expect(aufnahmen).toHaveCount(500);
    await dlg.getByTestId("dialog-mehr").click();
    await expect(aufnahmen).toHaveCount(620);
    await expect(dlg.getByTestId("dialog-mehr")).toHaveCount(0);
  });

  for (const [breite, dark, name] of [
    [1440, false, "01-projekt-1440-hell.png"],
    [1440, true, "01-projekt-1440-dunkel.png"],
    [390, false, "01-projekt-390-hell.png"],
  ] as const) {
    test(`Bild ${name} (LVA_SCREENSHOTS=1)`, async ({ page }) => {
      test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur auf Wunsch");
      fs.mkdirSync(SHOT_DIR, { recursive: true });
      await setup(page, {
        width: breite,
        height: breite === 390 ? 844 : 900,
        dark,
      });
      if (breite === 390) await waehleProjekt(page, "Finanzwissen");
      await expect(page.getByTestId("projekt-eintrag")).toHaveCount(9);
      await page.addStyleTag({
        content:
          "*,*::before,*::after{transition:none!important;animation:none!important}",
      });
      await page.waitForTimeout(400);
      await page.screenshot({
        path: path.join(SHOT_DIR, name),
        animations: "disabled",
      });
    });
  }

  test("Wechsel A zu B, während in A eine verzögerte Schaltung läuft: B behält seine Übersicht", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(
      page,
      `K.delay["projekt_kontext_schalten"] = 1500; K.delay["projekt_kontext_uebersicht:p-ki"] = 1800;`,
    );
    await schalter(page, "Telefonat Bank").click();
    await waehleProjekt(page, "KI-Strategie LRA");
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "23 Einträge",
    );
    // A schließt bei ca. 1,5 s ab, B antwortet erst bei ca. 1,8 s
    await page.waitForTimeout(2600);
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^[\d\s]+ von 128\s000 Token$/,
    );
    await expect(page.getByTestId("kontext-summe-text")).not.toHaveText(
      /^15\s000 von/,
    );
    await expect(page.getByTestId("kontext-anzahl")).toContainText(
      "Einträge als Kontext",
    );
    // A hat seine Schaltung nicht verloren
    await waehleProjekt(page, "Finanzwissen");
    await expect(schalter(page, "Telefonat Bank")).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  test("Bestand neu aufbauen in A, dann Wechsel nach B: B bleibt korrekt", async ({
    page,
  }) => {
    await setup(page, {
      vorher: (p) =>
        p.addInitScript(() => {
          (window as any).__kontext.lage = "defekt";
        }),
    });
    await kontextSetzen(
      page,
      `K.delay["projekt_dateien_neu_aufbauen"] = 800; K.delay["projekt_kontext_uebersicht:p-ki"] = 1400;`,
    );
    await page.getByTestId("lage-knopf").click();
    await page.getByTestId("neu-aufbauen-bestaetigen").click();
    await waehleProjekt(page, "KI-Strategie LRA");
    await expect(page.getByTestId("projekt-kopf-zahl")).toHaveText(
      "23 Einträge",
    );
    await page.waitForTimeout(2400);
    await expect(page.getByTestId("kontext-summe-text")).toHaveText(
      /^[\d\s]+ von 128\s000 Token$/,
    );
    await expect(page.getByTestId("banner-lage")).toHaveCount(0);
    await expect(page.getByTestId("banner-gefunden")).toHaveCount(0);
    expect(await aufrufe(page, "projekt_dateien_neu_aufbauen")).toHaveLength(1);
  });

  test("Kopiersperre übersteht einen Remount der Seite", async ({ page }) => {
    await setup(page);
    await kontextSetzen(
      page,
      `window.__pick = ["/home/test/Plan.md"]; K.delay["projekt_dateien_hinzufuegen"] = 2500;`,
    );
    await page.getByTestId("hinzufuegen").click();
    await page.getByRole("tab", { name: "Dateien kopieren" }).click();
    await page.getByTestId("dateien-waehlen").click();
    await expect(page.getByTestId("kopier-hinweis")).toBeVisible();
    await page.keyboard.press("Escape");
    await geheZu(page, "Verlauf");
    await expect(bereich(page)).toHaveCount(0);
    await geheZu(page, "Projekte");
    await expect(bereich(page)).toBeVisible();
    await page.getByTestId("hinzufuegen").click();
    await page.getByRole("tab", { name: "Dateien kopieren" }).click();
    await expect(page.getByTestId("dateien-waehlen")).toBeDisabled();
    await page.getByTestId("dateien-waehlen").click({ force: true });
    await expect(page.getByTestId("kopier-hinweis")).toBeVisible();
    await expect(page.getByTestId("projekt-eintrag")).toHaveCount(10, {
      timeout: 6000,
    });
    expect(await aufrufe(page, "projekt_dateien_hinzufuegen")).toHaveLength(1);
  });

  test("Dialog: verspätete Antwort nach Schließen und Wiederöffnen wird verworfen", async ({
    page,
  }) => {
    await setup(page);
    await kontextSetzen(
      page,
      `K.aufnahmen = 620; K.delay["meetings_list"] = 1500;`,
    );
    await page.getByTestId("hinzufuegen").click();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await kontextSetzen(page, `K.aufnahmen = 3; K.delay["meetings_list"] = 0;`);
    await page.getByTestId("hinzufuegen").click();
    const dlg = page.getByRole("dialog");
    const aufnahmen = dlg.getByRole("checkbox", { name: /Aufnahme \d+/ });
    await expect(aufnahmen).toHaveCount(3);
    await page.waitForTimeout(2000);
    await expect(aufnahmen).toHaveCount(3);
    await expect(dlg.getByTestId("dialog-mehr")).toHaveCount(0);
  });

  for (const [breite, hoehe] of [
    [1440, 900],
    [390, 844],
  ] as const) {
    test(`Dialoge vollständig sichtbar, nichts abgeschnitten (${breite}×${hoehe})`, async ({
      page,
    }) => {
      await setup(page, {
        width: breite,
        height: hoehe,
        vorher: (p) =>
          p.addInitScript(() => {
            (window as any).__kontext.lage = "defekt";
          }),
      });
      await kontextSetzen(
        page,
        `window.__pick = ["/home/test/Plan.md"]; K.delay["projekt_dateien_hinzufuegen"] = 60000;`,
      );
      const pruefen = async (wo: string) => {
        const dlg = page.getByRole("dialog");
        await expect(dlg).toBeVisible();
        await page.waitForTimeout(300);
        const m = await page.evaluate(() => {
          const d = document.querySelector<HTMLElement>('[role="dialog"]')!;
          const c = d.querySelector<HTMLElement>(".lv-dialog__content")!;
          const f = d.querySelector<HTMLElement>(".lv-dialog__actions");
          const mask = getComputedStyle(c).maskImage;
          const passt = c.scrollHeight <= c.clientHeight + 1;
          let letzteSichtbar = passt;
          if (!passt) {
            c.scrollTop = c.scrollHeight;
            const kind = c.lastElementChild as HTMLElement;
            letzteSichtbar =
              kind.getBoundingClientRect().bottom <=
              c.getBoundingClientRect().bottom + 1;
          }
          const l = d.querySelector<HTMLElement>(".pb-dlg-liste");
          let listeOk = true;
          if (l && l.lastElementChild) {
            l.scrollTop = l.scrollHeight;
            listeOk =
              l.lastElementChild.getBoundingClientRect().bottom <=
              l.getBoundingClientRect().bottom + 1;
          }
          const r = d.getBoundingClientRect();
          return {
            listeOk,
            passt,
            letzteSichtbar,
            mask,
            imFenster: r.top >= 0 && r.bottom <= window.innerHeight + 1,
            fussSichtbar: f
              ? f.getBoundingClientRect().bottom <= window.innerHeight + 1
              : true,
          };
        });
        expect(m.mask === "none" || m.mask === "", `${wo} Maske`).toBe(true);
        expect(m.letzteSichtbar, `${wo} letztes Element`).toBe(true);
        expect(m.listeOk, `${wo} letzte Listenzeile`).toBe(true);
        expect(m.imFenster, `${wo} im Fenster`).toBe(true);
        expect(m.fussSichtbar, `${wo} Fußleiste`).toBe(true);
      };
      const schliessen = async () => {
        await page.keyboard.press("Escape");
        await expect(page.getByRole("dialog")).toHaveCount(0);
      };
      // Projekt anlegen (bei 390 px steht zuerst nur die Liste)
      await page.getByTestId("projekt-neu").click();
      await pruefen("Projekt anlegen");
      await schliessen();
      if (breite === 390) await waehleProjekt(page, "Finanzwissen");
      for (const [name, wo] of [
        ["Depotauszug-2026.pdf", "Entfernen Datei"],
        ["Rich Dad Poor Dad", "Entfernen App-Inhalt"],
      ] as const) {
        await page
          .getByRole("button", { name: `Weitere Aktionen für ${name}` })
          .click();
        await page
          .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
          .click();
        await pruefen(wo);
        await schliessen();
      }
      await page.getByTestId("lage-knopf").click();
      await pruefen("Bestand neu aufbauen");
      await schliessen();
      await page.getByTestId("hinzufuegen").click();
      await expect(
        page.getByRole("checkbox", { name: /The Burning Room/ }),
      ).toBeVisible();
      await pruefen("Hinzufügen App");
      await page.getByRole("tab", { name: "Dateien kopieren" }).click();
      await page.getByTestId("dateien-waehlen").click();
      await expect(page.getByTestId("kopier-hinweis")).toBeVisible();
      await pruefen("Hinzufügen Dateien");
    });
  }

  // Späte Abschlüsse nach Schließen und Wiederöffnen: der neue Dialog bleibt offen und fehlerfrei.
  for (const fehler of [false, true]) {
    test(`Zuordnen ${fehler ? "scheitert" : "gelingt"} nach dem Wiederöffnen: neuer Dialog bleibt offen und fehlerfrei`, async ({
      page,
    }) => {
      await setup(page);
      await kontextSetzen(
        page,
        `K.delay["projekt_buch_zuordnen"] = 1200;${fehler ? ` K.fail.projekt_buch_zuordnen = "Zuordnen nicht möglich";` : ""}`,
      );
      await page.getByTestId("hinzufuegen").click();
      const dlg = page.getByRole("dialog");
      await dlg.getByRole("checkbox", { name: /The Burning Room/ }).check();
      await dlg.getByTestId("hinzufuegen-bestaetigen").click();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await page.getByTestId("hinzufuegen").click();
      await expect(
        page
          .getByRole("dialog")
          .getByRole("checkbox", { name: /The Burning Room/ }),
      ).toBeVisible();
      await page.waitForTimeout(1900);
      await expect(page.getByRole("dialog")).toHaveCount(1);
      await expect(page.getByRole("dialog").getByRole("alert")).toHaveCount(0);
      await expect(
        page.getByRole("dialog").getByTestId("hinzufuegen-bestaetigen"),
      ).toBeDisabled();
      if (!fehler)
        expect(await aufrufe(page, "projekt_buch_zuordnen")).toHaveLength(1);
    });

    test(`Kopie ${fehler ? "scheitert" : "gelingt"} nach dem Wiederöffnen: neuer Dialog bleibt offen und fehlerfrei`, async ({
      page,
    }) => {
      await setup(page);
      await kontextSetzen(
        page,
        `window.__pick = ["/home/test/Plan.md"]; K.delay["projekt_dateien_hinzufuegen"] = 1200;${fehler ? ` K.fail.projekt_dateien_hinzufuegen = "Kopieren nicht möglich";` : ""}`,
      );
      await page.getByTestId("hinzufuegen").click();
      await page.getByRole("tab", { name: "Dateien kopieren" }).click();
      await page.getByTestId("dateien-waehlen").click();
      await expect(page.getByTestId("kopier-hinweis")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await page.getByTestId("hinzufuegen").click();
      await expect(page.getByRole("dialog")).toHaveCount(1);
      await page.waitForTimeout(1900);
      await expect(page.getByRole("dialog")).toHaveCount(1);
      await expect(page.getByRole("dialog").getByRole("alert")).toHaveCount(0);
      if (!fehler) {
        expect(await aufrufe(page, "projekt_dateien_hinzufuegen")).toHaveLength(
          1,
        );
        await expect(page.getByTestId("projekt-eintrag")).toHaveCount(10);
      }
    });
  }

  interface Szene {
    name: string;
    schmal?: boolean;
    /** Szene wählt ihr Projekt selbst (kein vorgewähltes "Finanzwissen" bei 390 px). */
    eigeneWahl?: boolean;
    ziel?: string;
    vorher?: (p: Page) => Promise<void>;
    los?: (p: Page) => Promise<void>;
  }
  const flag = (f: () => void) => (p: Page) => p.addInitScript(f);
  const SZENEN: Szene[] = [
    {
      name: "02-banner-zugross",
      schmal: true,
      los: async (p) => {
        await p
          .getByRole("button", { name: "Form wählen: Kurzbuch" })
          .first()
          .click();
        await p.getByRole("menuitemradio", { name: /Volltext/ }).click();
        await expect(p.getByTestId("banner-zugross")).toBeVisible();
      },
    },
    {
      name: "03-banner-reset",
      schmal: true,
      vorher: flag(() => {
        (window as any).__kontext.reset = true;
      }),
      los: async (p) => {
        await expect(p.getByTestId("banner-reset")).toBeVisible();
      },
    },
    {
      name: "04-banner-gefunden",
      schmal: true,
      los: async (p) => {
        await expect(p.getByTestId("banner-gefunden")).toBeVisible();
      },
    },
    ...(["fehlt", "defekt", "neuer", "nicht_lesbar"] as const).map(
      (lage, i): Szene => ({
        name: `0${5 + i}-lage-${lage.replace("_", "-")}`,
        schmal: lage === "defekt",
        vorher: (p) =>
          p.addInitScript((l) => {
            (window as any).__kontext.lage = l;
          }, lage),
        los: async (p) => {
          await expect(p.getByTestId("banner-lage")).toBeVisible();
        },
      }),
    ),
    {
      name: "09-dialog-hinzufuegen-app",
      schmal: true,
      los: async (p) => {
        await p.evaluate(() => {
          (window as any).__kontext.aufnahmen = 3;
        });
        await p.getByTestId("hinzufuegen").click();
        await expect(
          p.getByRole("checkbox", { name: /The Burning Room/ }),
        ).toBeVisible();
        await p.getByRole("checkbox", { name: /The Burning Room/ }).check();
      },
    },
    {
      name: "10-dialog-hinzufuegen-dateien-kopiert",
      schmal: true,
      los: async (p) => {
        await p.evaluate(() => {
          const w = window as any;
          w.__pick = ["/home/test/Plan.md"];
          w.__kontext.delay.projekt_dateien_hinzufuegen = 60000;
        });
        await p.getByTestId("hinzufuegen").click();
        await p.getByRole("tab", { name: "Dateien kopieren" }).click();
        await p.getByTestId("dateien-waehlen").click();
        await expect(p.getByTestId("kopier-hinweis")).toBeVisible();
      },
    },
    {
      name: "11-dialog-entfernen-datei",
      schmal: true,
      los: async (p) => {
        await p
          .getByRole("button", {
            name: "Weitere Aktionen für Depotauszug-2026.pdf",
          })
          .click();
        await p
          .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
          .click();
        await expect(p.getByRole("dialog")).toBeVisible();
      },
    },
    {
      name: "12-dialog-entfernen-appinhalt",
      los: async (p) => {
        await p
          .getByRole("button", {
            name: "Weitere Aktionen für Rich Dad Poor Dad",
          })
          .click();
        await p
          .getByRole("menuitem", { name: "Aus Projekt entfernen …" })
          .click();
        await expect(p.getByRole("dialog")).toBeVisible();
      },
    },
    {
      name: "13-dialog-bestand-neu-aufbauen",
      vorher: flag(() => {
        (window as any).__kontext.lage = "defekt";
      }),
      los: async (p) => {
        await p.getByTestId("lage-knopf").click();
        await expect(p.getByRole("dialog")).toBeVisible();
      },
    },
    {
      name: "14-dialog-projekt-anlegen",
      los: async (p) => {
        await p.getByTestId("projekt-neu").click();
        await p
          .getByRole("textbox", { name: /Name/ })
          .fill("Wohnungssuche 2027");
      },
    },
    {
      name: "15-dialog-ladefehler",
      vorher: flag(() => {
        (window as any).__kontext.fail.pages_list = "Seiten nicht lesbar";
      }),
      los: async (p) => {
        await p.getByTestId("hinzufuegen").click();
        await expect(p.getByTestId("dialog-ladefehler")).toBeVisible();
      },
    },
    {
      name: "16-leerzustand",
      schmal: true,
      eigeneWahl: true,
      los: async (p) => {
        await waehleProjekt(p, "Neue Ideen");
        await expect(p.getByTestId("projekt-leer")).toBeVisible();
      },
    },
    {
      name: "17-vorlesen-gruppe",
      ziel: "Vorlesen",
      eigeneWahl: true,
      los: async (p) => {
        await expect(
          p.locator('[data-testid="projekte-gruppe"]:visible'),
        ).toBeVisible();
      },
    },
    {
      name: "18-gespraech-gruppe",
      eigeneWahl: true,
      los: async (p) => {
        await geheZu(p, "Start");
        await expect(
          p.locator('[data-testid="projekte-gruppe"]:visible'),
        ).toBeVisible();
      },
    },
  ];
  for (const sz of SZENEN) {
    for (const [breite, dark, ende] of [
      [1440, false, "1440-hell"],
      [1440, true, "1440-dunkel"],
      ...(sz.schmal ? ([[390, false, "390-hell"]] as const) : []),
    ] as const) {
      test(`Bild ${sz.name}-${ende} (LVA_SCREENSHOTS=1)`, async ({ page }) => {
        test.skip(!process.env.LVA_SCREENSHOTS, "Bilder nur auf Wunsch");
        fs.mkdirSync(SHOT_DIR, { recursive: true });
        await setup(page, {
          width: breite,
          height: breite === 390 ? 844 : 900,
          dark,
          vorher: sz.vorher,
          ziel: sz.ziel,
        });
        if (breite === 390 && !sz.eigeneWahl)
          await waehleProjekt(page, "Finanzwissen");
        await sz.los?.(page);
        await page.addStyleTag({
          content:
            "*,*::before,*::after{transition:none!important;animation:none!important}",
        });
        await page.waitForTimeout(500);
        await page.screenshot({
          path: path.join(SHOT_DIR, `${sz.name}-${ende}.png`),
          animations: "disabled",
        });
      });
    }
  }
});
