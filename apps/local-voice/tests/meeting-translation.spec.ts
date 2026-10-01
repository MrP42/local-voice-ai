import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import * as path from "node:path";
import { installLiveMock } from "./recLiveMock";
import {
  calls,
  emitMeeting,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
import {
  DE_TEXT,
  EN_TEXT,
  installTranslationMock,
  releaseTranslation,
  type TranslationMockOptions,
} from "./translationMock";

// G5 (Goal Issues-Abschluss #70): Mehrsprachigkeit gegen die Tauri-Attrappe.
//  1. Sprach-Chip im Kopf (erkannte Sprache, Herkunft) und Korrektur mit Angebot zur
//     Neu-Transkription mit dem passenden Modell.
//  2. „Übersetzen nach …“ legt eine NEUE Fassung an; das Original bleibt aktiv und waehlbar.
//  3. Vergleich Satz fuer Satz mit Zeitmarken (kein Wort-Diff), markierte Saetze.
//  4. Protokoll und KI-Notizen mit Wahl der Grundlage und Ausgabesprache; beides im Kopf
//     und im Info-Dialog; Herkunft der Uebersetzung.
//  5. Zusatzfix: Aufgaben-Kontrollkaestchen haben einen Namen (axe `label`).
// Die Logik (Erkennung, Modellwahl, Treuepruefung, Prompts) pruefen die Rust-Tests.

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const SHOT_DIR = path.resolve(
  process.cwd(),
  "../../koordination/issues-abschluss/screens",
);
/** Bilder nur auf Wunsch (LVA_SCREENSHOTS=1), sonst schreibt jeder Lauf sie neu. */
const shoot = async (page: Page, name: string) => {
  if (!process.env.LVA_SCREENSHOTS) return;
  await page.waitForTimeout(300);
  await page.screenshot({
    path: path.join(SHOT_DIR, `${name}.png`),
    animations: "disabled",
  });
};

const MINUTES_WITH_BASIS = [
  "# Protokoll: Kundentermin Stadtwerke Kaiserslautern",
  "",
  "**Datum:** 28.09.2026 · **Dauer:** 59 Min. · **Vorlage:** Allgemein",
  "**Grundlage:** Übersetzung (Deutsch) · Protokoll auf Englisch",
  "",
  "## Aufgaben",
  "- [ ] Angebot an Frau Becker schicken (Wer: Thomas)",
  "- [x] Tarifblatt 2027 anfordern",
].join("\n");

const setup = async (
  page: Page,
  options: TranslationMockOptions = {},
  width = 1366,
  height = 768,
) => {
  await installLiveMock(page);
  await installTranslationMock(page, options);
  await page.addInitScript((body) => {
    const w = window as any;
    const minutes = w.__documents.find((d: any) => d.id === "p1");
    minutes.body = body;
  }, MINUTES_WITH_BASIS);
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

const chooseMenu = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

/** Eintrag der react-select-Liste waehlen (Liste haengt an body). */
const pickOption = async (page: Page, container: string, name: string) => {
  await page.getByTestId(container).locator("input").first().click();
  await page.getByRole("option", { name, exact: true }).click();
};

const chip = (page: Page) => page.getByTestId("language-chip");
const variantChip = (page: Page) => page.getByTestId("variant-chip");

const clickItem = async (page: Page, name: string | RegExp) => {
  await page
    .getByRole("menuitem", { name })
    .evaluate((el) => (el as HTMLElement).click());
};

/** Uebersetzung nach Deutsch ueber das Menue starten; wartet auf die neue Fassung. */
const translateToGerman = async (page: Page) => {
  await chooseMenu(page, "menu-translate");
  const dialog = page.getByRole("dialog", { name: "Übersetzen nach …" });
  await expect(dialog).toBeVisible();
  await dialog.getByTestId("translate-start").click();
  await expect(dialog).toHaveCount(0);
  await expect(variantChip(page)).toBeVisible();
};

// ---------------------------------------------------------------------------
// 1. Sprach-Chip und Korrektur
// ---------------------------------------------------------------------------

test.describe("Sprach-Chip", () => {
  test("zeigt die erkannte Sprache mit Herkunft im Kopf", async ({ page }) => {
    await setup(page);
    await expect(chip(page)).toBeVisible();
    await expect(chip(page)).toHaveAttribute("data-language", "en");
    await expect(chip(page)).toHaveAttribute("data-source", "probe");
    await expect(chip(page)).toContainText("Englisch");
    await expect(chip(page)).toHaveAttribute(
      "title",
      /Spracherkennung des Modells/,
    );
    await expect(chip(page)).toHaveAccessibleName(/Englisch/);
    await shoot(page, "g5-sprach-chip");
  });

  test("der Kopf bleibt mit dem Chip kompakt (nicht hoeher als 120 px)", async ({
    page,
  }) => {
    await setup(page);
    const box = await page.getByTestId("rec-detail-head").boundingBox();
    expect(box!.height).toBeLessThanOrEqual(120);
    const row = await page.getByTestId("rec-detail-chips-main").boundingBox();
    expect(row!.height).toBeLessThanOrEqual(26);
  });

  test("schmal steht nur der Code, der Name bleibt im Namen fuer Hilfsmittel", async ({
    page,
  }) => {
    await setup(page, {}, 480, 800);
    await expect(chip(page)).toContainText("en");
    await expect(chip(page)).toHaveAccessibleName(/Englisch/);
  });

  test("der Dialog nennt Herkunft und Modell", async ({ page }) => {
    await setup(page);
    await chip(page).click();
    const dialog = page.getByRole("dialog", {
      name: "Sprache der Besprechung",
    });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("language-source")).toContainText(
      "Erkannt: Spracherkennung des Modells",
    );
    await expect(dialog.getByTestId("language-source")).toContainText(
      "Parakeet TDT 0.6B v3",
    );
    await expect(dialog.getByTestId("language-suggestion")).toHaveCount(0);
    await shoot(page, "g5-sprach-dialog");
  });

  test("Korrektur: speichern setzt die Sprache und der Chip folgt", async ({
    page,
  }) => {
    await setup(page);
    await chip(page).click();
    await pickOption(page, "language-select", "Französisch");
    await page.getByTestId("language-save").click();
    expect((await calls(page, "meetings_set_language"))[0].args).toEqual({
      meetingId: "m2",
      language: "fr",
    });
    await expect(chip(page)).toHaveAttribute("data-language", "fr");
    await expect(chip(page)).toHaveAttribute("data-source", "user");
    await expect(chip(page)).toContainText("Französisch");
    expect(await calls(page, "meetings_retranscribe")).toHaveLength(0);
  });

  test("Korrektur mit Angebot: Neu-Transkription mit dem passenden Modell, neue Fassung", async ({
    page,
  }) => {
    await setup(page);
    await chip(page).click();
    await pickOption(page, "language-select", "Französisch");
    await page.getByTestId("language-save-retranscribe").click();
    // Der Dialog der Neu-Transkription ist mit Sprache und installiertem Modell vorbelegt.
    const dialog = page.getByRole("dialog", { name: "Neu transkribieren" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("retranscribe-language")).toContainText(
      "Französisch",
    );
    await expect(dialog.getByTestId("retranscribe-suggestion")).toContainText(
      "Qwen3-ASR 1.7B (installiert, ausgewählt)",
    );
    await shoot(page, "g5-neu-transkribieren");
    await dialog.getByTestId("retranscribe-start").click();
    expect((await calls(page, "meetings_retranscribe"))[0].args).toEqual({
      meetingId: "m2",
      modelId: "qwen3-asr-1.7b",
      language: "fr",
    });
  });

  test("deckt das Modell die Sprache nicht ab, steht das mit dem besseren Modell im Dialog", async ({
    page,
  }) => {
    await setup(page, { modelNotCovering: true });
    await expect(chip(page)).toBeVisible();
    await chip(page).click();
    const hint = page.getByTestId("language-suggestion");
    await expect(hint).toContainText(
      "Parakeet v2 (nur Deutsch) deckt Englisch nicht ab",
    );
    await expect(hint).toContainText("Qwen3-ASR 1.7B (installiert)");
  });

  test("ohne Audio laesst sich nur die Angabe aendern, nicht neu transkribieren", async ({
    page,
  }) => {
    await installLiveMock(page);
    await installTranslationMock(page);
    await page.addInitScript(() => {
      const w = window as any;
      w.__meetings = w.__meetings.map((m: any) =>
        m.id === "m2" ? { ...m, mic_audio_path: null } : m,
      );
    });
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, "m2");
    await expect(chip(page)).toBeVisible();
    await chip(page).click();
    await expect(page.getByTestId("language-save-retranscribe")).toBeDisabled();
    await expect(page.getByTestId("language-save")).toBeEnabled();
  });
});

// ---------------------------------------------------------------------------
// 2. Uebersetzung als neue Fassung
// ---------------------------------------------------------------------------

test.describe("Uebersetzen", () => {
  test("der Dialog schlaegt Deutsch vor und startet fuer die aktive Fassung", async ({
    page,
  }) => {
    await setup(page);
    await chooseMenu(page, "menu-translate");
    const dialog = page.getByRole("dialog", { name: "Übersetzen nach …" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("translate-target")).toContainText(
      "Deutsch",
    );
    // Eine Fassung: keine Auswahl der Quelle.
    await expect(dialog.getByTestId("translate-source")).toHaveCount(0);
    await expect(dialog.getByTestId("translate-hint")).toContainText(
      "Zahlen, Eigennamen und die Satzanzahl",
    );
    await shoot(page, "g5-uebersetzen-dialog");
    await dialog.getByTestId("translate-start").click();
    expect((await calls(page, "transcript_variant_translate"))[0].args).toEqual(
      {
        meetingId: "m2",
        sourceVariantId: "v1",
        targetLanguage: "de",
      },
    );
  });

  test("die Ausgangssprache ist als Ziel gesperrt, andere Ziele sind waehlbar", async ({
    page,
  }) => {
    await setup(page);
    await chooseMenu(page, "menu-translate");
    await page.getByTestId("translate-target").locator("input").first().click();
    await expect(
      page.getByRole("option", { name: "Englisch", exact: true }),
    ).toHaveAttribute("aria-disabled", "true");
    await page.getByRole("option", { name: "Spanisch", exact: true }).click();
    await page.getByTestId("translate-start").click();
    expect(
      (await calls(page, "transcript_variant_translate"))[0].args,
    ).toMatchObject({
      targetLanguage: "es",
    });
  });

  test("erzeugt eine neue Fassung; das Original bleibt aktiv und unveraendert", async ({
    page,
  }) => {
    await setup(page);
    await expect(variantChip(page)).toHaveCount(0);
    await translateToGerman(page);
    // Hinweis mit Weg zum Vergleich; das Transkript zeigt weiter das Original.
    await expect(
      page
        .locator("[data-sonner-toast]")
        .filter({ hasText: "Fassung v2 (Deutsch) ist fertig." }),
    ).toBeVisible();
    await expect(variantChip(page)).toHaveText(
      "Fassung v1 · Eigene Transkription (en)",
    );
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      "Good morning, this is Anna from Siemens.",
    );
    await variantChip(page).click();
    await expect(
      page.getByRole("menuitem", {
        name: /^v1 · Eigene Transkription \(en\) · 6 Segmente \(aktiv\)$/,
      }),
    ).toBeDisabled();
    await expect(
      page.getByRole("menuitem", {
        name: "v2 · Übersetzung (de) · 6 Segmente · 1 markiert",
      }),
    ).toBeEnabled();
    await shoot(page, "g5-fassungen");
  });

  test("Wechsel zwischen Original und Uebersetzung verliert nichts", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    await variantChip(page).click();
    await clickItem(page, /^v2 · Übersetzung/);
    await expect(variantChip(page)).toHaveText("Fassung v2 · Übersetzung (de)");
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      DE_TEXT[0],
    );
    await expect(page.getByTestId("transcript-scroll")).not.toContainText(
      EN_TEXT[0],
    );
    await variantChip(page).click();
    await clickItem(page, /^v1 · Eigene Transkription/);
    await expect(variantChip(page)).toHaveText(
      "Fassung v1 · Eigene Transkription (en)",
    );
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      EN_TEXT[0],
    );
    // Der Sprach-Chip folgt der aktiven Fassung: erneut gelesen.
    await expect(chip(page)).toBeVisible();
  });

  test("auch aus dem Fassungs-Chip laesst sich uebersetzen", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    await variantChip(page).click();
    await clickItem(page, "Übersetzen nach …");
    const dialog = page.getByRole("dialog", { name: "Übersetzen nach …" });
    await expect(dialog).toBeVisible();
    // Zwei Fassungen: die Quelle ist waehlbar, vorbelegt die aktive.
    await expect(dialog.getByTestId("translate-source")).toContainText(
      "v1 · Eigene Transkription (en)",
    );
  });

  test("aus dem Sprach-Dialog fuehrt ein Knopf zur Uebersetzung", async ({
    page,
  }) => {
    await setup(page);
    await chip(page).click();
    await page.getByTestId("language-translate").click();
    await expect(
      page.getByRole("dialog", { name: "Übersetzen nach …" }),
    ).toBeVisible();
  });

  test("ein Fehler des Modells zeigt einen Hinweis und legt keine Fassung an", async ({
    page,
  }) => {
    await setup(page, { failTranslate: true });
    await chooseMenu(page, "menu-translate");
    await page.getByTestId("translate-start").click();
    await expect(
      page
        .locator("[data-sonner-toast]")
        .filter({ hasText: "Das Sprachmodell hat nicht geantwortet" }),
    ).toBeVisible();
    await expect(variantChip(page)).toHaveCount(0);
  });

  test("waehrend der Uebersetzung steht der Fortschritt in der Bedienspalte und die Aktionen ruhen", async ({
    page,
  }) => {
    await setup(page, { holdTranslate: true });
    await chooseMenu(page, "menu-translate");
    await page.getByTestId("translate-start").click();
    await emitMeeting(page, {
      kind: "progress",
      meeting_id: "m2",
      phase: "translation",
      done: 1,
      total: 3,
      elapsed_ms: 8000,
      eta_ms: 16000,
      state: "running",
      pausable: true,
    });
    const panel = page.getByTestId("job-panel");
    await expect(panel).toHaveAttribute("data-phase", "translation");
    await expect(page.getByTestId("job-phase")).toHaveText("Übersetzung");
    await expect(page.getByTestId("job-percent")).toHaveText("33 %");
    // Eine Uebersetzung ist keine Transkription: das Transkript waechst nicht mit.
    await expect(page.getByTestId("autoscroll-toggle")).toHaveCount(0);
    await shoot(page, "g5-uebersetzung-laeuft");
    await releaseTranslation(page);
    await emitMeeting(page, { kind: "job_ended", meeting_id: "m2" });
    await expect(variantChip(page)).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// 3. Vergleich Satz fuer Satz
// ---------------------------------------------------------------------------

test.describe("Vergleich Original und Uebersetzung", () => {
  const openCompare = async (page: Page) => {
    await setup(page);
    await translateToGerman(page);
    await variantChip(page).click();
    await page
      .getByRole("menuitem", { name: "Vergleichen" })
      .evaluate((el) => (el as HTMLElement).click());
    await expect(page.getByTestId("sentence-compare")).toBeVisible();
  };

  test("zeigt Satz fuer Satz mit Zeitmarken, nicht als Wort-Diff", async ({
    page,
  }) => {
    await openCompare(page);
    const rows = page.getByTestId("sentence-row");
    await expect(rows).toHaveCount(6);
    await expect(rows.first().getByTestId("sentence-time")).toHaveText("0:01");
    await expect(rows.nth(1).getByTestId("sentence-time")).toHaveText("0:07");
    await expect(rows.first().getByTestId("sentence-original")).toHaveText(
      EN_TEXT[0],
    );
    await expect(rows.first().getByTestId("sentence-translation")).toHaveText(
      DE_TEXT[0],
    );
    await expect(page.getByTestId("sentence-head-original")).toHaveText(
      "Original Englisch",
    );
    await expect(page.getByTestId("sentence-head-translation")).toHaveText(
      "Übersetzung Deutsch",
    );
    // Kein Wort-Diff zwischen zwei Sprachen.
    await expect(page.locator('[data-testid="diff-rows"]')).toHaveCount(0);
    await expect(page.locator("del, ins")).toHaveCount(0);
    await shoot(page, "g5-satz-vergleich");
  });

  test("der Satz mit abweichender Zahl ist markiert, mit Grund und ohne verworfen zu sein", async ({
    page,
  }) => {
    await openCompare(page);
    await expect(page.getByTestId("sentence-summary")).toHaveText(
      "1 Satz ist markiert: bitte prüfen.",
    );
    const flagged = page.locator(
      '[data-testid="sentence-row"][data-flagged="true"]',
    );
    await expect(flagged).toHaveCount(1);
    await expect(flagged.getByTestId("sentence-reasons")).toHaveText(
      "Zahl weicht ab",
    );
    // Der Text des Modells bleibt stehen.
    await expect(flagged.getByTestId("sentence-translation")).toHaveText(
      DE_TEXT[1],
    );
    await expect(flagged.getByTestId("sentence-original")).toHaveText(
      EN_TEXT[1],
    );
  });

  test("nur markierte Saetze zeigen", async ({ page }) => {
    await openCompare(page);
    await page.getByTestId("sentence-only-flagged").check();
    await expect(page.getByTestId("sentence-row")).toHaveCount(1);
    await page.getByTestId("sentence-only-flagged").uncheck();
    await expect(page.getByTestId("sentence-row")).toHaveCount(6);
  });

  test("Original und Uebersetzung lassen sich nicht zusammenfuehren", async ({
    page,
  }) => {
    await openCompare(page);
    await expect(page.getByTestId("compare-merge")).toBeDisabled();
  });

  test("breit stehen Original und Uebersetzung nebeneinander in einer Zeile", async ({
    page,
  }) => {
    await setup(page, {}, 1920, 1050);
    await translateToGerman(page);
    await variantChip(page).click();
    await page
      .getByRole("menuitem", { name: "Vergleichen" })
      .evaluate((el) => (el as HTMLElement).click());
    await expect(page.getByTestId("sentence-compare")).toBeVisible();
    const row = page.getByTestId("sentence-row").first();
    const original = await row.getByTestId("sentence-original").boundingBox();
    const translated = await row
      .getByTestId("sentence-translation")
      .boundingBox();
    // Gleiche Zeile, Spalten nebeneinander und etwa gleich breit.
    expect(Math.abs(translated!.y - original!.y)).toBeLessThanOrEqual(2);
    expect(translated!.x).toBeGreaterThan(original!.x + original!.width - 1);
    expect(Math.abs(translated!.width - original!.width)).toBeLessThanOrEqual(
      4,
    );
    // Die Spaltenköpfe stehen über den Spalten.
    await expect(page.getByTestId("sentence-head-original")).toBeVisible();
    await shoot(page, "g5-satz-vergleich-breit");
  });

  test("schmal stehen Original und Uebersetzung untereinander", async ({
    page,
  }) => {
    await setup(page, {}, 480, 800);
    await translateToGerman(page);
    await variantChip(page).click();
    await page
      .getByRole("menuitem", { name: "Vergleichen" })
      .evaluate((el) => (el as HTMLElement).click());
    const row = page.getByTestId("sentence-row").first();
    const original = await row.getByTestId("sentence-original").boundingBox();
    const translated = await row
      .getByTestId("sentence-translation")
      .boundingBox();
    expect(translated!.y).toBeGreaterThan(original!.y);
    // Keine waagerechte Scrollbar der Seite.
    const overflow = await page.evaluate(
      () => document.scrollingElement!.scrollWidth - innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  });

  test("zwei Fassungen ohne Uebersetzungs-Beziehung bleiben beim Wort-Diff", async ({
    page,
  }) => {
    await setup(page, { withMerged: true });
    await translateToGerman(page);
    await variantChip(page).click();
    await page
      .getByRole("menuitem", { name: "Vergleichen" })
      .evaluate((el) => (el as HTMLElement).click());
    // A = v1 (aktiv), B = die zusammengefuehrte Fassung: Wort-Diff wie bisher.
    await expect(page.getByTestId("diff-rows")).toBeVisible();
    await expect(page.getByTestId("sentence-compare")).toHaveCount(0);
    await expect(page.getByTestId("compare-merge")).toBeEnabled();
    // Wechsel auf die Uebersetzung als B: Satz fuer Satz.
    await pickOption(
      page,
      "compare-b",
      "v3 · Übersetzung (de) · 6 Segmente · 1 markiert",
    );
    await expect(page.getByTestId("sentence-compare")).toBeVisible();
    await expect(page.getByTestId("diff-rows")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// 4. Protokoll und KI-Notizen: Grundlage und Ausgabesprache
// ---------------------------------------------------------------------------

test.describe("Grundlage und Ausgabesprache", () => {
  const openRegen = async (page: Page) => {
    await chooseMenu(page, "menu-regen-template");
    const dialog = page.getByRole("dialog", {
      name: "Neu erzeugen mit Vorlage",
    });
    await expect(dialog).toBeVisible();
    return dialog;
  };

  test("Standard: aktive Fassung und die Sprache der App", async ({ page }) => {
    await setup(page);
    await translateToGerman(page);
    const dialog = await openRegen(page);
    await expect(dialog.getByTestId("docbasis-variant")).toContainText(
      "v1 · Eigene Transkription (en) · 6 Segmente (aktiv)",
    );
    await expect(dialog.getByTestId("docbasis-language")).toContainText(
      "Deutsch",
    );
    await shoot(page, "g5-grundlage-dialog");
  });

  test("Protokoll mit Uebersetzung als Grundlage und englischer Ausgabe", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    const dialog = await openRegen(page);
    await pickOption(
      page,
      "docbasis-variant",
      "v2 · Übersetzung (de) · 6 Segmente · 1 markiert",
    );
    await pickOption(page, "docbasis-language", "Englisch");
    await dialog.getByTestId("regen-minutes-go").click();
    expect((await calls(page, "meetings_generate_minutes"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      basis: { variant_id: "v2", output_language: "en" },
    });
  });

  test("KI-Notizen mit derselben Wahl", async ({ page }) => {
    await setup(page);
    await translateToGerman(page);
    const dialog = await openRegen(page);
    await pickOption(
      page,
      "docbasis-variant",
      "v2 · Übersetzung (de) · 6 Segmente · 1 markiert",
    );
    await pickOption(page, "docbasis-language", "Französisch");
    await dialog.getByTestId("regen-notes-go").click();
    expect((await calls(page, "meeting_notes_enhance"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      basis: { variant_id: "v2", output_language: "fr" },
    });
  });

  test("die letzte Wahl der Sprache gilt beim naechsten Mal und fuer Laeufe ohne Dialog", async ({
    page,
  }) => {
    await setup(page);
    let dialog = await openRegen(page);
    await pickOption(page, "docbasis-language", "Spanisch");
    await dialog.getByTestId("regen-minutes-go").click();
    // Ohne Dialog (Menue "Protokoll neu erzeugen"): aktive Fassung, letzte Wahl.
    await chooseMenu(page, "menu-regen-minutes");
    const all = await calls(page, "meetings_generate_minutes");
    expect(all[1].args).toEqual({
      meetingId: "m2",
      templateId: null,
      basis: { variant_id: null, output_language: "es" },
    });
    dialog = await openRegen(page);
    await expect(dialog.getByTestId("docbasis-language")).toContainText(
      "Spanisch",
    );
  });

  test("ohne gespeicherte Wahl gilt die Sprache der App bei Laeufen ohne Dialog", async ({
    page,
  }) => {
    await setup(page);
    await chooseMenu(page, "menu-regen-notes");
    expect((await calls(page, "meeting_notes_enhance"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      basis: { variant_id: null, output_language: "de" },
    });
  });

  test('"Wie die Grundlage" schickt keine Sprache mit', async ({ page }) => {
    await setup(page);
    const dialog = await openRegen(page);
    await pickOption(page, "docbasis-language", "Wie die Grundlage");
    await dialog.getByTestId("regen-minutes-go").click();
    expect(
      (await calls(page, "meetings_generate_minutes"))[0].args,
    ).toMatchObject({
      basis: { variant_id: null, output_language: null },
    });
  });

  test("Grundlage und Ausgabesprache stehen im Protokoll-Kopf und im Info-Dialog", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    // Ein Lauf der Attrappe legt die Angaben der Dokumentversion an.
    const dialog = await openRegen(page);
    await pickOption(
      page,
      "docbasis-variant",
      "v2 · Übersetzung (de) · 6 Segmente · 1 markiert",
    );
    await pickOption(page, "docbasis-language", "Englisch");
    await dialog.getByTestId("regen-minutes-go").click();
    // Reiter Protokoll: die Kopfzeile steht im Text (das Backend schreibt sie dorthin).
    await expect(page.getByTestId("mid-panel-minutes")).toContainText(
      "Grundlage: Übersetzung (Deutsch) · Protokoll auf Englisch",
    );
    await shoot(page, "g5-protokoll-kopf");
    // Info-Dialog: Grundlage des Protokolls.
    await page.getByTestId("meeting-details-open").click();
    await expect(page.getByTestId("minutes-basis")).toHaveText(
      "Grundlage: Übersetzung (Deutsch) · Protokoll auf Englisch",
    );
    await expect(page.getByTestId("notes-basis")).toHaveCount(0);
  });

  test("KI-Notizen weisen Grundlage und Sprache im Kopf aus", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    const dialog = await openRegen(page);
    await pickOption(
      page,
      "docbasis-variant",
      "v2 · Übersetzung (de) · 6 Segmente · 1 markiert",
    );
    await pickOption(page, "docbasis-language", "Deutsch");
    await dialog.getByTestId("regen-notes-go").click();
    await page.getByRole("tab", { name: "KI-Notizen", exact: true }).click();
    await expect(page.getByTestId("enhanced-basis")).toHaveCount(0);
    await page.getByTestId("meeting-details-open").click();
    await expect(page.getByTestId("notes-basis")).toHaveText(
      "Grundlage: Übersetzung (Deutsch) · KI-Notizen auf Deutsch",
    );
  });
});

// ---------------------------------------------------------------------------
// 5. Herkunft der Uebersetzung
// ---------------------------------------------------------------------------

test.describe("Herkunft der Uebersetzung", () => {
  test("Modell, Token, Ausgangsfassung und Zeitpunkt am Transkript der Uebersetzung", async ({
    page,
  }) => {
    await setup(page);
    await translateToGerman(page);
    await variantChip(page).click();
    await clickItem(page, /^v2 · Übersetzung/);
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      DE_TEXT[0],
    );
    await page
      .getByTestId("prov-area-transcript")
      .click({ button: "right", position: { x: 12, y: 12 } });
    await clickItem(page, "Herkunft");
    const dialog = page.getByTestId("provenance-dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("prov-operation")).toContainText(
      "translation",
    );
    await expect(dialog.getByTestId("prov-model")).toContainText("Gemma 4 E4B");
    await expect(dialog.getByTestId("prov-tokens")).toContainText(
      "1800 ein · 900 aus",
    );
    await expect(dialog.getByTestId("prov-sources")).toContainText(
      "v1 stt (en)",
    );
    await expect(dialog.getByTestId("prov-time")).toContainText("2026");
    await shoot(page, "g5-herkunft");
  });
});

// ---------------------------------------------------------------------------
// 6. Zusatzfix: Aufgaben-Kontrollkaestchen haben einen Namen
// ---------------------------------------------------------------------------

test.describe("Aufgaben-Kontrollkaestchen im Protokoll", () => {
  test("tragen den Aufgabentext als Namen", async ({ page }) => {
    await setup(page);
    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    const boxes = page
      .getByTestId("mid-panel-minutes")
      .locator('input[type="checkbox"]');
    await expect(boxes).toHaveCount(2);
    await expect(boxes.first()).toHaveAccessibleName(
      "Angebot an Frau Becker schicken (Wer: Thomas)",
    );
    await expect(boxes.nth(1)).toHaveAccessibleName(
      "Tarifblatt 2027 anfordern",
    );
    await expect(boxes.nth(1)).toBeChecked();
  });

  test("axe findet keine Verletzung der Regel 'label' mehr", async ({
    page,
  }) => {
    await setup(page);
    await page.getByRole("tab", { name: "Protokoll", exact: true }).click();
    await expect(page.getByTestId("mid-panel-minutes")).toContainText(
      "Tarifblatt 2027",
    );
    const results = await new AxeBuilder({ page })
      .include('[data-testid="mid-panel-minutes"]')
      .analyze();
    const labelViolations = results.violations.filter((v) => v.id === "label");
    expect(labelViolations).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// 7. Barrierefreiheit der neuen Teile
// ---------------------------------------------------------------------------

test.describe("Barrierefreiheit", () => {
  const critical = (results: Awaited<ReturnType<AxeBuilder["analyze"]>>) =>
    results.violations.filter(
      (v) => v.impact === "critical" || v.impact === "serious",
    );

  test("Chip, Sprach-Dialog und Uebersetzen-Dialog ohne kritische Befunde", async ({
    page,
  }) => {
    await setup(page);
    await chip(page).click();
    const dialog = page.getByRole("dialog", {
      name: "Sprache der Besprechung",
    });
    await expect(dialog).toBeVisible();
    expect(
      critical(
        await new AxeBuilder({ page }).include('[role="dialog"]').analyze(),
      ),
    ).toEqual([]);
    await page.keyboard.press("Escape");
    await chooseMenu(page, "menu-translate");
    await expect(
      page.getByRole("dialog", { name: "Übersetzen nach …" }),
    ).toBeVisible();
    expect(
      critical(
        await new AxeBuilder({ page }).include('[role="dialog"]').analyze(),
      ),
    ).toEqual([]);
  });

  test("der Satz-Vergleich ohne kritische Befunde", async ({ page }) => {
    await setup(page);
    await translateToGerman(page);
    await variantChip(page).click();
    await page
      .getByRole("menuitem", { name: "Vergleichen" })
      .evaluate((el) => (el as HTMLElement).click());
    await expect(page.getByTestId("sentence-compare")).toBeVisible();
    expect(
      critical(
        await new AxeBuilder({ page })
          .include('[data-testid="sentence-compare"]')
          .analyze(),
      ),
    ).toEqual([]);
  });

  test("der Sprach-Chip ist per Tastatur erreichbar und oeffnet mit Enter", async ({
    page,
  }) => {
    await setup(page);
    await chip(page).focus();
    await page.keyboard.press("Enter");
    await expect(
      page.getByRole("dialog", { name: "Sprache der Besprechung" }),
    ).toBeVisible();
  });
});
