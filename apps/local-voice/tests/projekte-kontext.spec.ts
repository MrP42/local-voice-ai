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

    await geheZu(page, "Chat");
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
});
