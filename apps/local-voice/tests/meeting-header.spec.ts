import { test, expect, type Page } from "@playwright/test";
import {
  PROGRESS,
  TITLE_M2,
  TITLE_M3,
  VIEWPORTS,
  calls,
  emitMeeting,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";

// Aufnahmen-Oberflaeche (Goal aufnahmen-ui, M4): kompakter Detailkopf, Symbolzeile
// mit Tooltips, Menue "☰", Details-Dialog, Neu-transkribieren-Dialog und der
// Startdialog der Aufnahme. AK5 (Kopf <= 120 px, Vollinfo im Dialog,
// Neu-Transkription im Menue, Tooltips mit Maus und Tastatur), Teil von AK9.

test.beforeEach(async ({ page }) => {
  await installRecMock(page);
});

const head = (page: Page) => page.getByTestId("rec-detail-head");

const openM2 = async (page: Page, width = 1366, height = 768) => {
  await openRecordings(page, width, height);
  await pickMeeting(page, "m2");
  await expect(page.locator('[data-segment-index="0"]')).toBeVisible();
};

/** Menue "☰" oeffnen und einen Eintrag waehlen (per JS-Klick: das Menue schliesst bei Scroll). */
const chooseMenu = async (page: Page, testId: string) => {
  await page.getByTestId("meeting-menu").click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.getByTestId(testId).evaluate((el) => (el as HTMLElement).click());
};

const PARTICIPANTS = [
  "Anna Berg",
  "Bernd Alt",
  "Clara Neu",
  "Dirk Voss",
  "Eva Holm",
].map((name, i) => ({
  human_id: `h${i}`,
  name,
  email: null,
  company: null,
  role: i === 0 ? "organizer" : "attendee",
  source: "calendar",
  is_self: false,
  meeting_count: 1,
}));

// ---------------------------------------------------------------------------
// AK5: Kopf
// ---------------------------------------------------------------------------

test.describe("Detailkopf", () => {
  for (const label of ["1920", "1366", "480"] as const) {
    test(`AK5: Kopf vom Anfang bis zur Unterkante der Reiter <= 120 px (${label})`, async ({
      page,
    }) => {
      const vp = VIEWPORTS[label];
      await openM2(page, vp.width, vp.height);
      const box = (await head(page).boundingBox())!;
      const tabs = (await head(page).getByRole("tablist").boundingBox())!;
      expect(tabs.y + tabs.height - box.y, label).toBeLessThanOrEqual(120);
      expect(box.height, label).toBeLessThanOrEqual(120);
    });
  }

  test("AK5: auch mit Teilnehmenden, Projekten und laufender Verarbeitung bleibt der Kopf <= 120 px", async ({
    page,
  }) => {
    await page.addInitScript((list) => {
      (window as any).__participants = { m2: list };
    }, PARTICIPANTS);
    await openM2(page);
    await emitMeeting(page, { ...PROGRESS, meeting_id: "m2" });
    await expect(page.getByTestId("status-chip")).toHaveAttribute(
      "data-state",
      "processing",
    );
    // G4: ein Chip "5 Teilnehmende: Anna Berg, Bernd Alt, Clara Neu, +2"; der
    // Rest und die volle Liste stehen im Tooltip bzw. im Popover.
    const chip = page.getByTestId("participants-chip");
    await expect(chip).toContainText("5 Teilnehmende");
    await expect(head(page)).toContainText("+2");
    const box = (await head(page).boundingBox())!;
    expect(box.height).toBeLessThanOrEqual(120);
    await chip.click();
    const rows = page.getByTestId("participant-row");
    await expect(rows).toHaveCount(5);
    await expect(rows.first().getByTestId("participant-name")).toHaveText(
      "Anna Berg",
    );
  });

  test("Kopf: Titel, Chips und Reiter; der grosse Statusblock und der Neu-transkribieren-Kasten entfallen", async ({
    page,
  }) => {
    await openM2(page);
    const content = page.getByTestId("rec-content");
    await expect(
      content.getByRole("heading", { name: TITLE_M2 }),
    ).toBeVisible();
    await expect(page.getByTestId("status-chip")).toHaveText("Fertig");
    await expect(page.getByTestId("source-chip")).toHaveAttribute(
      "title",
      "Datei-Import",
    );
    await expect(page.getByTestId("duration-chip")).toHaveText("59 Min.");
    await expect(page.getByTestId("project-chip")).toContainText(
      "Geschäftlich",
    );
    await expect(page.getByTestId("project-chip")).toContainText("+1");
    // G4: Mitte = Transkript und Protokoll; Notizen und KI-Notizen rechts unten.
    for (const name of ["Transkript", "Protokoll"]) {
      await expect(
        head(page).getByRole("tab", { name, exact: true }),
      ).toBeVisible();
    }
    for (const name of ["Notizen", "KI-Notizen", "Fragen"]) {
      await expect(
        page.getByTestId("rec-lower").getByRole("tab", { name, exact: true }),
      ).toBeVisible();
    }
    // Die Rastertabelle und der Kasten sind weg; beides steht im Dialog bzw. Menue.
    await expect(content.getByText("Einwilligung bestätigt")).toHaveCount(0);
    await expect(
      content.getByRole("button", { name: "Neu transkribieren" }),
    ).toHaveCount(0);
    // Die Vorlagenwahl steht nicht als Auswahl in den Reitern Notizen / KI-Notizen
    // und Protokoll (sie steht im Menue).
    const lower = page.getByTestId("rec-lower");
    await expect(lower.locator(".app-select__control")).toHaveCount(0);
    await lower.getByRole("tab", { name: "KI-Notizen" }).click();
    await expect(lower.locator(".app-select__control")).toHaveCount(0);
    await head(page).getByRole("tab", { name: "Protokoll" }).click();
    await expect(content.locator(".app-select__control")).toHaveCount(0);
  });

  test("Titel: Klick benennt um, Enter speichert, Escape verwirft", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("meeting-title").click();
    const input = page.getByTestId("meeting-title-input");
    await expect(input).toBeFocused();
    await input.fill("Neuer Titel");
    await input.press("Escape");
    await expect(input).toHaveCount(0);
    expect(await calls(page, "meetings_rename")).toHaveLength(0);
    await expect(page.getByTestId("meeting-title")).toHaveText(TITLE_M2);

    await page.getByTestId("meeting-title").click();
    await input.fill("Neuer Titel");
    await input.press("Enter");
    await expect(page.getByTestId("meeting-title")).toHaveText("Neuer Titel");
    expect((await calls(page, "meetings_rename"))[0].args).toEqual({
      meetingId: "m2",
      title: "Neuer Titel",
    });
    // Die Liste zieht nach.
    await expect(page.locator('[data-meeting-id="m2"]')).toContainText(
      "Neuer Titel",
    );
  });

  test("Titel: F2 benennt um (Titel fokussiert und global)", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("meeting-title").focus();
    await page.keyboard.press("F2");
    await expect(page.getByTestId("meeting-title-input")).toBeFocused();
    await page.keyboard.press("Escape");
    // Ohne Fokus im Titel, aber auch nicht in einem Eingabefeld.
    await page
      .getByTestId("rec-detail-chips")
      .click({ position: { x: 3, y: 1 } });
    await page.keyboard.press("F2");
    await expect(page.getByTestId("meeting-title-input")).toBeFocused();
  });

  test("Details-Dialog: Status, Quelle, Datei, Spuren, Beginn, Dauer, Sprache, Modell, Einwilligung, Audio-Loeschdatum, Segmente", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("meeting-details-open").click();
    const dialog = page.getByRole("dialog", { name: "Details" });
    await expect(dialog).toBeVisible();
    const row = (key: string) => dialog.getByTestId(`details-${key}`);
    await expect(row("title")).toHaveText(TITLE_M2);
    await expect(row("status")).toHaveText("Fertig");
    await expect(row("source")).toHaveText("Datei-Import");
    await expect(row("file")).toHaveText("SWK-Lastgang-2026-09-28.m4a");
    await expect(row("tracks")).toHaveText("Datei (ein Kanal)");
    await expect(row("projects")).toHaveText("Geschäftlich, Kunde Stadtwerke");
    await expect(row("started")).toContainText("2026");
    await expect(row("duration")).toHaveText("59:00");
    await expect(row("language")).toHaveText("Deutsch");
    await expect(row("model")).toContainText("Besprechungsmodell");
    await expect(row("consent")).toContainText("2026");
    await expect(row("retention")).toContainText("2026");
    await expect(row("segments")).toHaveText("60");
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    // Auch das Menue fuehrt zum Dialog.
    await chooseMenu(page, "menu-details");
    await expect(page.getByRole("dialog", { name: "Details" })).toBeVisible();
  });

  test("Projekt-Chip und Menue: In Projekt verschieben oeffnet die Auswahl mit den aktuellen Projekten", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("project-chip").click();
    const dialog = page.getByRole("dialog", {
      name: "In Projekt verschieben …",
    });
    await expect(dialog).toBeVisible();
    const box = (name: string) => dialog.getByRole("checkbox", { name });
    await expect(box("Geschäftlich")).toHaveAttribute("aria-checked", "true");
    await expect(box("Kunde Stadtwerke")).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await expect(box("Privat")).toHaveAttribute("aria-checked", "false");
    await box("Privat").click();
    await dialog.getByRole("button", { name: "Speichern" }).click();
    await expect(dialog).toHaveCount(0);
    expect((await calls(page, "meetings_set_folders")).at(-1)!.args).toEqual({
      meetingId: "m2",
      folderIds: ["f2", "f3", "f1"],
    });
    await expect(page.getByTestId("project-chip")).toContainText("Privat");
    // Ueber das Menue derselbe Dialog.
    await chooseMenu(page, "menu-move");
    await expect(
      page.getByRole("dialog", { name: "In Projekt verschieben …" }),
    ).toBeVisible();
  });
});

// ---------------------------------------------------------------------------
// AK5 / AK9: Symbolzeile, Menue, Tooltips
// ---------------------------------------------------------------------------

test.describe("Symbolzeile und Menue", () => {
  const ICONS = [
    ["export-open", "Exportieren"],
    ["followup-open", "Follow-up-Mail"],
    ["copy-transcript", "Kopieren"],
    ["people-open", "Personen"],
    ["chat-toggle", "Fragen"],
  ] as const;

  test("die Symbolzeile hat gleich grosse Knoepfe mit Namen, Rolle und Tastaturkuerzel", async ({
    page,
  }) => {
    await openM2(page);
    const toolbar = page.getByTestId("rec-actions");
    await expect(toolbar).toHaveAttribute("role", "toolbar");
    const sizes: string[] = [];
    for (const [id, name] of ICONS) {
      const button = toolbar.getByTestId(id);
      await expect(button).toHaveAttribute("aria-label", name);
      const b = (await button.boundingBox())!;
      sizes.push(`${Math.round(b.width)}x${Math.round(b.height)}`);
    }
    expect(new Set(sizes)).toEqual(new Set(["36x36"]));
    await expect(toolbar.getByTestId("chat-toggle")).toHaveAttribute(
      "aria-keyshortcuts",
      "Control+J",
    );
    // G4: das Menue steht im Kopf neben "Details", nicht mehr in der Symbolzeile.
    await expect(toolbar.getByTestId("meeting-menu")).toHaveCount(0);
    const menu = head(page).getByTestId("meeting-menu");
    await expect(menu).toHaveAttribute("aria-label", "Menü");
    await expect(menu).toHaveAttribute("aria-haspopup", "menu");
    const m = (await menu.boundingBox())!;
    const info = (await head(page)
      .getByTestId("meeting-details-open")
      .boundingBox())!;
    expect(Math.round(m.width)).toBe(36);
    expect(m.x).toBeGreaterThan(info.x);
    expect(Math.abs(m.y - info.y)).toBeLessThan(2);
  });

  test("AK5: Tooltips mit Maus - Name und Kurzerklaerung, verbunden per aria-describedby", async ({
    page,
  }) => {
    await openM2(page);
    for (const [id, name] of [...ICONS, ["meeting-menu", "Menü"] as const]) {
      const button = page.getByTestId(id);
      await button.hover();
      const tip = page.getByRole("tooltip");
      await expect(tip, id).toBeVisible();
      await expect(tip.locator("strong")).toHaveText(name);
      // Zweite Zeile: die Kurzerklaerung.
      await expect(tip.locator("div.text-xs")).not.toHaveText("");
      const tipId = await tip.getAttribute("id");
      await expect(button).toHaveAttribute("aria-describedby", tipId!);
      await page.mouse.move(5, 5);
      await expect(tip).toHaveCount(0);
    }
  });

  test("AK5/AK9: Tooltips per Tastatur - Tab wandert durch die Symbole, jedes zeigt sofort sein Tooltip", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("export-open").focus();
    for (let i = 0; i < ICONS.length; i++) {
      const [id, name] = ICONS[i];
      await expect(page.getByTestId(id), id).toBeFocused();
      const tip = page.getByRole("tooltip");
      await expect(tip.locator("strong")).toHaveText(name);
      await expect(page.getByTestId(id)).toHaveAttribute(
        "aria-describedby",
        (await tip.getAttribute("id"))!,
      );
      if (i < ICONS.length - 1) await page.keyboard.press("Tab");
    }
    // Esc schliesst das Tooltip.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("tooltip")).toHaveCount(0);
  });

  test("Titel und Info-Symbol haben ebenfalls ein Tooltip (Maus und Tastatur)", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("meeting-details-open").focus();
    await expect(page.getByRole("tooltip").locator("strong")).toHaveText(
      "Details",
    );
    // Tab: weiter zum Menue neben dem Info-Symbol, mit sofortigem Tooltip.
    await page.keyboard.press("Tab");
    await expect(page.getByTestId("meeting-menu")).toBeFocused();
    await expect(page.getByRole("tooltip").locator("strong")).toHaveText(
      "Menü",
    );
    await page.keyboard.press("Escape");
    await page.getByTestId("meeting-title").hover();
    await expect(page.getByRole("tooltip").locator("strong")).toHaveText(
      "Umbenennen",
    );
    await expect(page.getByTestId("meeting-title")).toHaveAttribute(
      "aria-describedby",
      (await page.getByRole("tooltip").getAttribute("id"))!,
    );
  });

  test("Menue: alle Eintraege in der vorgesehenen Reihenfolge, Pfeiltasten und Esc", async ({
    page,
  }) => {
    await openM2(page);
    await page.getByTestId("meeting-menu").click();
    const items = page.getByRole("menuitem");
    await expect(items).toHaveCount(14);
    const labels = (await items.allTextContents()).map((t) => t.trim());
    expect(labels).toEqual([
      "Neu transkribieren …",
      "Übersetzen nach …",
      "KI-Notizen neu erzeugen",
      "Protokoll neu erzeugen",
      "Neu erzeugen mit Vorlage …",
      "Vorlage wählen …",
      "Vorlagen verwalten …",
      "Sprecher benennen …",
      "UmbenennenF2",
      "In Projekt verschieben …",
      "Nur Text kopieren",
      "Transkript als Datei …",
      "Details …",
      "Besprechung löschen …",
    ]);
    await expect(items.first()).toBeFocused();
    await page.keyboard.press("ArrowDown");
    await expect(items.nth(1)).toBeFocused();
    await page.keyboard.press("End");
    await expect(items.last()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await expect(page.getByTestId("meeting-menu")).toBeFocused();
  });

  test("Menue: Umbenennen startet die Bearbeitung des Titels", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-rename");
    await expect(page.getByTestId("meeting-title-input")).toBeFocused();
  });

  test("Menue: Vorlage waehlen oeffnet einen Dialog mit der Vorlagenwahl (nicht doppelt als Karte)", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-template");
    const dialog = page.getByRole("dialog", { name: "Vorlage wählen" });
    await expect(dialog.locator(".app-select__control")).toHaveCount(1);
    await expect(
      dialog.getByRole("button", { name: "Vorlagen verwalten …" }),
    ).toBeVisible();
    // Die Aufnahmezeile rechts traegt keine Vorlagenwahl mehr.
    await expect(
      page.getByTestId("rec-controls").getByTestId("record-template"),
    ).toHaveCount(0);
  });

  test("Menue: KI-Notizen und Protokoll neu erzeugen starten den Lauf und zeigen den Reiter", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-regen-notes");
    await expect
      .poll(async () => (await calls(page, "meeting_notes_enhance")).length)
      .toBe(1);
    expect((await calls(page, "meeting_notes_enhance"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      // G5: aktive Fassung, Ausgabesprache = Sprache der App
      basis: { variant_id: null, output_language: "de" },
    });
    await expect(
      page.getByTestId("rec-lower").getByRole("tab", { name: "KI-Notizen" }),
    ).toHaveAttribute("aria-selected", "true");
    await chooseMenu(page, "menu-regen-minutes");
    await expect
      .poll(async () => (await calls(page, "meetings_generate_minutes")).length)
      .toBe(1);
    expect((await calls(page, "meetings_generate_minutes"))[0].args).toEqual({
      meetingId: "m2",
      templateId: null,
      // G5: aktive Fassung, Ausgabesprache = Sprache der App
      basis: { variant_id: null, output_language: "de" },
    });
    await expect(
      head(page).getByRole("tab", { name: "Protokoll" }),
    ).toHaveAttribute("aria-selected", "true");
  });

  test("Menue: Nur Text kopieren legt das Transkript ohne Zeitmarken in die Zwischenablage", async ({
    page,
    baseURL,
  }) => {
    await page
      .context()
      .grantPermissions(["clipboard-read", "clipboard-write"], {
        origin: baseURL,
      });
    await openM2(page);
    await chooseMenu(page, "menu-copy-plain");
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text.startsWith("Guten Morgen zusammen")).toBe(true);
    expect(text).not.toContain("[0:04]");
    // Das Kopier-Symbol liefert die Fassung mit Zeitmarken.
    await page.getByTestId("copy-transcript").click();
    await expect(page.getByTestId("copy-transcript")).toHaveAttribute(
      "aria-label",
      "Kopiert",
    );
    const withMeta = await page.evaluate(() => navigator.clipboard.readText());
    expect(withMeta).toContain("[0:04]: Guten Morgen zusammen");
  });

  test("Menue: Loeschen fragt nach, Abbrechen loescht nichts, Bestaetigen loescht und schliesst die Ansicht", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-delete");
    const dialog = page.getByRole("dialog", { name: "Besprechung löschen?" });
    await expect(dialog).toBeVisible();
    // Fusszeile (das Kreuz oben traegt denselben Namen).
    await dialog.getByRole("button", { name: "Abbrechen" }).last().click();
    expect(await calls(page, "meetings_delete")).toHaveLength(0);
    await expect(head(page)).toBeVisible();

    await chooseMenu(page, "menu-delete");
    await page.getByTestId("meeting-delete-confirm").click();
    await expect
      .poll(async () => (await calls(page, "meetings_delete")).length)
      .toBe(1);
    expect((await calls(page, "meetings_delete"))[0].args).toEqual({
      meetingId: "m2",
    });
    await expect(head(page)).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Neu transkribieren (Dialog, B17-Hinweis)
// ---------------------------------------------------------------------------

test.describe("Neu transkribieren", () => {
  test("Dialog mit Modellwahl und dem korrekten Hinweis; Start ruft den Befehl und schliesst", async ({
    page,
  }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    const dialog = page.getByRole("dialog", { name: "Neu transkribieren" });
    await expect(dialog).toBeVisible();
    await expect(
      dialog.getByText("Eingestelltes Besprechungsmodell"),
    ).toBeVisible();
    // B17: das Transkript wird waehrend des Laufs ersetzt, bei einem Stopp bleibt der neue Teil.
    await expect(dialog.getByTestId("retranscribe-hint")).toHaveText(
      "Das bisherige Transkript bleibt als Fassung erhalten. Während der Lauf fortschreitet, zeigt die Ansicht den neuen Text; erst am Ende wird er die aktive Fassung. Stoppst du vorher, bleibt das bisherige Transkript aktiv. Bei langen Aufzeichnungen dauert das eine Weile.",
    );
    expect(await calls(page, "meetings_retranscribe")).toHaveLength(0);
    await dialog.getByTestId("retranscribe-start").click();
    await expect(dialog).toHaveCount(0);
    expect((await calls(page, "meetings_retranscribe"))[0].args).toEqual({
      meetingId: "m2",
      modelId: null,
      // G5: Sprache automatisch erkennen
      language: null,
    });
  });

  test("Abbrechen startet nichts", async ({ page }) => {
    await openM2(page);
    await chooseMenu(page, "menu-retranscribe");
    await page
      .getByRole("dialog", { name: "Neu transkribieren" })
      .getByRole("button", { name: "Abbrechen" })
      .click();
    expect(await calls(page, "meetings_retranscribe")).toHaveLength(0);
  });

  test("der Eintrag ist gesperrt, solange eine Verarbeitung laeuft, und ohne Audio", async ({
    page,
  }) => {
    await openM2(page);
    await emitMeeting(page, { ...PROGRESS, meeting_id: "m2" });
    await expect(page.getByTestId("job-panel")).toBeVisible();
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByTestId("menu-retranscribe")).toBeDisabled();
    await expect(page.getByTestId("menu-regen-notes")).toBeDisabled();
    await page.keyboard.press("Escape");

    // m3 hat keine Audiodatei.
    await pickMeeting(page, "m3");
    await expect(
      page.getByTestId("rec-content").getByRole("heading", { name: TITLE_M3 }),
    ).toBeVisible();
    await page.getByTestId("meeting-menu").click();
    await expect(page.getByTestId("menu-retranscribe")).toBeDisabled();
  });
});

// ---------------------------------------------------------------------------
// Bedienung: Aufnahme starten, Import, Link, laufende Aufnahme
// ---------------------------------------------------------------------------

test.describe("Bedienung oben rechts", () => {
  test("Zeile: Aufnahme starten (primaer), Import, Link einfuegen (aktiv seit A2, mit Tooltip)", async ({
    page,
  }) => {
    await openRecordings(page, 1366, 768);
    const row = page.getByTestId("rec-start");
    await expect(
      row.getByRole("button", { name: /Aufnahme starten/ }),
    ).toBeVisible();
    await expect(row.getByTestId("import-open")).toHaveAttribute(
      "aria-label",
      "Datei importieren",
    );
    const link = row.getByTestId("link-open");
    // A2 (#65): der Link-Knopf ist aktiv und oeffnet den YouTube-Dialog.
    await expect(link).toBeEnabled();
    const b = (await link.boundingBox())!;
    await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2);
    const tip = page.getByRole("tooltip");
    await expect(tip.locator("strong")).toHaveText("Link einfügen");
    await expect(tip).toContainText("YouTube-Link als Quelle anlegen");
    // Keine Formularfelder mehr in der Bedienspalte.
    await expect(
      page
        .getByTestId("rec-controls")
        .getByPlaceholder("Titel der Besprechung"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("rec-controls").getByTestId("capture-system"),
    ).toHaveCount(0);
  });

  test("Startdialog: Titel, Projekt, Vorlage, System-Audio, mehrere Personen und Einwilligung; gestartet wird erst nach der Bestaetigung", async ({
    page,
  }) => {
    await openRecordings(page, 1366, 768);
    await page.getByRole("button", { name: /Aufnahme starten/ }).click();
    const dialog = page.getByRole("dialog", { name: "Aufnahme starten" });
    await expect(dialog).toBeVisible();
    // Einwilligung (§ 201 StGB) und Hinweis fuer den Meeting-Chat stehen im Dialog.
    await expect(dialog).toContainText("Einwilligung erforderlich");
    await expect(dialog).toContainText("201 StGB");
    await expect(dialog.getByTestId("consent-chat-notice")).toBeVisible();
    await expect(dialog.getByTestId("record-template")).toBeVisible();
    await expect(dialog.getByTestId("capture-system")).toBeChecked();
    await expect(dialog.getByTestId("diarize-mic")).not.toBeChecked();
    expect(await calls(page, "meetings_start")).toHaveLength(0);

    await dialog
      .getByPlaceholder("Titel der Besprechung")
      .fill("Kick-off Sommerfest");
    await dialog
      .getByTestId("start-project")
      .locator(".app-select__control")
      .click();
    await page.getByRole("option", { name: "Privat", exact: true }).click();
    await dialog
      .getByTestId("record-template")
      .locator(".app-select__control")
      .click();
    await page
      .getByRole("option", { name: "Kundengespräch / Vertrieb", exact: true })
      .click();
    // ToggleSwitch: das Eingabefeld ist unsichtbar (sr-only), der sichtbare Schalter
    // liegt darueber; wie in script-check.spec.ts mit force bedient.
    await dialog.getByTestId("diarize-mic").check({ force: true });
    // Bis hierher nichts gestartet.
    expect(await calls(page, "meetings_start")).toHaveLength(0);

    await dialog
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
    await expect(dialog).toHaveCount(0);
    await expect
      .poll(async () => (await calls(page, "meetings_start")).length)
      .toBe(1);
    expect((await calls(page, "meetings_start"))[0].args).toEqual({
      title: "Kick-off Sommerfest",
      consentConfirmed: true,
      captureSystem: true,
      targetMeetingId: null,
    });
    await expect
      .poll(async () => (await calls(page, "meetings_set_folders")).length)
      .toBe(1);
    expect((await calls(page, "meetings_set_folders"))[0].args).toEqual({
      meetingId: "m-neu",
      folderIds: ["f1"],
    });
    expect((await calls(page, "meetings_set_template"))[0].args).toEqual({
      meetingId: "m-neu",
      templateId: "builtin:vertrieb",
    });
    expect((await calls(page, "meetings_set_diarize_mic"))[0].args).toEqual({
      meetingId: "m-neu",
      enabled: true,
    });
    await expect(page.getByTestId("rec-live")).toBeVisible();
  });

  test("Startdialog: Abbrechen und Esc starten nichts", async ({ page }) => {
    await openRecordings(page, 1366, 768);
    await page.getByRole("button", { name: /Aufnahme starten/ }).click();
    const dialog = page.getByRole("dialog", { name: "Aufnahme starten" });
    await dialog.getByRole("button", { name: "Abbrechen" }).last().click();
    await expect(dialog).toHaveCount(0);
    await page.getByRole("button", { name: /Aufnahme starten/ }).click();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    expect(await calls(page, "meetings_start")).toHaveLength(0);
  });

  test("Import: das Symbol oeffnet die Dateiwahl, danach die Einwilligung; erst die Bestaetigung importiert", async ({
    page,
  }) => {
    await openRecordings(page, 1366, 768);
    await page.evaluate(() => ((window as any).__pick = "C:/Audio/Kunde.m4a"));
    await page.getByTestId("import-open").click();
    const dialog = page.getByRole("dialog", {
      name: "Einwilligung erforderlich",
    });
    await expect(dialog).toContainText("Kunde.m4a");
    expect(await calls(page, "meetings_import_file")).toHaveLength(0);
    await dialog
      .getByRole("button", { name: "Alle Beteiligten haben zugestimmt" })
      .click();
    await expect
      .poll(async () => (await calls(page, "meetings_import_file")).length)
      .toBe(1);
    expect((await calls(page, "meetings_import_file"))[0].args).toEqual({
      path: "C:/Audio/Kunde.m4a",
      consentConfirmed: true,
      targetMeetingId: null,
    });
  });

  test("laufende Aufnahme: Zeile mit rotem Punkt, Uhr, Pegel, Pause, Stopp und Hinweis - der Chip zeigt den Laufzustand, nicht 'Besprechungen'", async ({
    page,
  }) => {
    await installRecMock(page, { recording: true });
    await openRecordings(page, 1366, 768);
    const live = page.getByTestId("rec-live");
    await expect(live).toBeVisible();
    const chip = live.getByTestId("rec-state-chip");
    await expect(chip).toHaveText("Aufnahme läuft");
    await expect(live).toHaveAttribute("data-state", "recording");
    await expect(live).not.toContainText("Besprechungen");
    // Die Uhr startet bei der Aufnahmeposition (12:34) und zaehlt weiter.
    await expect(live.getByTestId("rec-clock")).toHaveText(/^12:3[4-9]$/);
    await expect(live.getByTestId("rec-levels")).toBeVisible();
    await expect(live.getByTestId("rec-pause")).toHaveAttribute(
      "aria-label",
      "Pause",
    );
    await expect(live.getByTestId("rec-stop")).toHaveAttribute(
      "aria-label",
      "Beenden",
    );
    await expect(
      live.getByTestId("recording-chat-notice-copy"),
    ).toHaveAttribute("aria-label", "Hinweis kopieren");
    // Kein zweiter Start waehrend der Aufnahme.
    await expect(page.getByTestId("rec-start")).toHaveCount(0);

    await emitMeeting(page, {
      kind: "state",
      meeting_id: "m1",
      status: "recording",
      paused: true,
    });
    await expect(chip).toHaveText("Pausiert");
    await expect(live).toHaveAttribute("data-state", "paused");
    await expect(live.getByTestId("rec-resume")).toHaveAttribute(
      "aria-label",
      "Weiter",
    );
  });

  test("laufende Aufnahme: Pegel kommen aus den Ereignissen, Stopp ruft den Befehl", async ({
    page,
  }) => {
    await installRecMock(page, { recording: true });
    await openRecordings(page, 1366, 768);
    await emitMeeting(page, { kind: "levels", mic: 0.5, system: 0.25 });
    const bars = page.getByTestId("rec-levels").locator("> div > div");
    await expect(bars.first()).toHaveAttribute("style", /width: 50%/);
    await expect(bars.nth(1)).toHaveAttribute("style", /width: 25%/);
    await page.getByTestId("rec-stop").click();
    await expect
      .poll(async () => (await calls(page, "meetings_stop")).length)
      .toBe(1);
  });

  test("laufende Aufnahme: die Zeile passt in die Spaltenbreite (480)", async ({
    page,
  }) => {
    await installRecMock(page, { recording: true });
    await openRecordings(page, 480, 800);
    const live = (await page.getByTestId("rec-live").boundingBox())!;
    const controls = (await page.getByTestId("rec-controls").boundingBox())!;
    expect(live.x + live.width).toBeLessThanOrEqual(
      controls.x + controls.width + 1,
    );
    expect(live.height).toBeLessThanOrEqual(60);
  });
});

// ---------------------------------------------------------------------------
// Wiedergabe und Fortschritt in der Spalte
// ---------------------------------------------------------------------------

test.describe("Wiedergabe und Fortschritt", () => {
  for (const label of ["1366", "480"] as const) {
    test(`der Player passt in die Spaltenbreite und zeigt das Tempo (${label})`, async ({
      page,
    }) => {
      const vp = VIEWPORTS[label];
      await openM2(page, vp.width, vp.height);
      const player = page.getByTestId("rec-player");
      const pb = (await player.boundingBox())!;
      const controls = (await page.getByTestId("rec-controls").boundingBox())!;
      expect(pb.x + pb.width).toBeLessThanOrEqual(
        controls.x + controls.width + 1,
      );
      const rate = player.getByRole("button", { name: "Playback speed" });
      await expect(rate).toBeVisible();
      const rb = (await rate.boundingBox())!;
      expect(rb.x + rb.width).toBeLessThanOrEqual(
        controls.x + controls.width + 1,
      );
      // Der Dateiname steht nicht mehr doppelt ueber dem Player.
      await expect(player).not.toContainText("SWK-Lastgang");
    });
  }

  test("Fortschritt: eine Zeile mit Phase, Balken, Prozent und Symbolknoepfen fuer Pause und Stopp; Chip im Kopf zeigt denselben Stand", async ({
    page,
  }) => {
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, "m3");
    await emitMeeting(page, PROGRESS);
    const panel = page.getByTestId("job-panel");
    await expect(panel).toBeVisible();
    await expect(panel.getByTestId("job-phase")).toHaveText("Transkription");
    await expect(panel.getByTestId("job-percent")).toHaveText("42 %");
    const pause = panel.getByTestId("job-pause");
    await expect(pause).toHaveAttribute("aria-label", "Pausieren");
    const stop = panel.getByTestId("job-stop");
    await expect(stop).toHaveAttribute("aria-label", "Stoppen");
    const [pb, sb, bar] = await Promise.all([
      pause.boundingBox(),
      stop.boundingBox(),
      panel.getByRole("progressbar").boundingBox(),
    ]);
    // Alles auf einer Zeile (gleiche Mitte), hoechstens ~70 px Gesamthoehe.
    expect(Math.abs(pb!.y - sb!.y)).toBeLessThanOrEqual(1);
    expect(bar!.y).toBeGreaterThan(pb!.y - 2);
    expect(bar!.y + bar!.height).toBeLessThan(pb!.y + pb!.height + 2);
    expect((await panel.boundingBox())!.height).toBeLessThanOrEqual(80);
    // Chip im Kopf: Phase, Balken, Prozent.
    const chip = page.getByTestId("status-chip");
    await expect(chip).toHaveAttribute("data-state", "processing");
    await expect(chip).toContainText("Transkription");
    await expect(chip).toContainText("42 %");
    // Tooltip am Pausenknopf nennt Name und Erklaerung.
    await pause.hover();
    await expect(page.getByRole("tooltip").locator("strong")).toHaveText(
      "Pausieren",
    );
  });
});
