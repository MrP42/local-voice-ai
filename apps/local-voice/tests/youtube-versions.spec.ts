import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import {
  calls,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
import { installYoutubeMock, YT_MEETING_ID } from "./youtubeMock";
import { installVariantsMock, type VariantsMockOptions } from "./variantsMock";

// A3 (#66, AK4/AK5/AK6): Fassungen des Transkripts, Vergleich, Zusammenfuehren und
// der Rechtsklick „Herkunft“ gegen die Tauri-Attrappe. Die Logik selbst pruefen die
// Rust-Tests (`cargo test --lib variants:: merge:: youtube::`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const setup = async (page: Page, options: VariantsMockOptions = {}) => {
  await installRecMock(page);
  await installYoutubeMock(page);
  await installVariantsMock(page, options);
  await openRecordings(page, 1366, 768);
  await pickMeeting(page, YT_MEETING_ID);
  await expect(page.getByTestId("variant-chip")).toBeVisible();
};

const chip = (page: Page) => page.getByTestId("variant-chip");

/** Eintrag eines Kontextmenues (schliesst bei Scroll: Klick ueber das Element). */
const menuItem = (page: Page, name: string | RegExp) =>
  page.getByRole("menuitem", { name });

const clickItem = async (page: Page, name: string | RegExp) => {
  await menuItem(page, name).evaluate((el) => (el as HTMLElement).click());
};

const tab = (page: Page, name: string) =>
  page.getByRole("tab", { name, exact: true });

// ---------------------------------------------------------------------------
// Fassungs-Chip
// ---------------------------------------------------------------------------

test.describe("Fassungs-Chip", () => {
  test("zeigt die aktive Fassung und listet alle", async ({ page }) => {
    await setup(page);
    await expect(chip(page)).toHaveText(
      "Fassung v1 · Untertitel (manuell) (de)",
    );
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      "über den Lastgang.",
    );
    await chip(page).click();
    await expect(page.getByRole("menu", { name: "Fassungen" })).toBeVisible();
    await expect(menuItem(page, /^v1 · .*\(aktiv\)$/)).toBeDisabled();
    await expect(
      menuItem(page, "v2 · Eigene Transkription (de) · 2 Segmente"),
    ).toBeEnabled();
    await expect(menuItem(page, "Vergleichen")).toBeVisible();
  });

  test("Fassung wählen setzt das aktive Transkript", async ({ page }) => {
    await setup(page);
    await chip(page).click();
    await clickItem(page, "v2 · Eigene Transkription (de) · 2 Segmente");
    const activated = await calls(page, "transcript_variant_activate");
    expect(activated).toHaveLength(1);
    expect(activated[0].args).toEqual({ variantId: "vb" });
    await expect(chip(page)).toHaveText(
      "Fassung v2 · Eigene Transkription (de)",
    );
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      "ueber den Lastgang von Stadtwerken.",
    );
    // Nur noch die andere ist wählbar.
    await chip(page).click();
    await expect(menuItem(page, /^v1 · /)).toBeEnabled();
    await expect(menuItem(page, /^v2 · .*\(aktiv\)$/)).toBeDisabled();
  });

  test("mit nur einer Fassung gibt es keinen Reiter Vergleich", async ({
    page,
  }) => {
    await setup(page, { single: true });
    await expect(tab(page, "Vergleich")).toHaveCount(0);
    await chip(page).click();
    await expect(menuItem(page, "Vergleichen")).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Vergleich und Zusammenführen
// ---------------------------------------------------------------------------

test.describe("Vergleich", () => {
  test("markiert Einfügungen und Löschungen wortweise", async ({ page }) => {
    await setup(page);
    await tab(page, "Vergleich").click();
    await expect(page.getByTestId("compare-view")).toBeVisible();
    // Nur die geänderte Zeile erscheint; die zweite ist gleich.
    await expect(page.getByTestId("diff-row")).toHaveCount(1);
    const row = page.getByTestId("diff-row");
    await expect(row.locator('del[data-diff="del"]')).toHaveText("über");
    await expect(row.locator('ins[data-diff="ins"]').nth(0)).toHaveText(
      "ueber",
    );
    await expect(row.locator('ins[data-diff="ins"]').nth(1)).toHaveText(
      "von Stadtwerken.",
    );
    // Der unveränderte Teil steht unmarkiert dazwischen.
    await expect(row).toContainText("Wir sprechen heute");
    await expect(row).toContainText("den Lastgang");
    // Zeitmarke der Zeile (erstes Segment ab 0:01).
    await expect(row).toContainText("0:01");
    // Sichtbar auch ohne Farbe: durchgestrichen bzw. unterstrichen.
    const decoration = await row
      .locator("del")
      .evaluate((el) => getComputedStyle(el).textDecorationLine);
    expect(decoration).toBe("line-through");
    const insDecoration = await row
      .locator("ins")
      .first()
      .evaluate((el) => getComputedStyle(el).textDecorationLine);
    expect(insDecoration).toBe("underline");
  });

  test("der Menüeintrag Vergleichen öffnet den Reiter", async ({ page }) => {
    await setup(page);
    await chip(page).click();
    await clickItem(page, "Vergleichen");
    await expect(page.getByTestId("compare-view")).toBeVisible();
    await expect(tab(page, "Vergleich")).toHaveAttribute(
      "aria-selected",
      "true",
    );
    // Rechts bleibt die aktive Fassung stehen.
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      "über den Lastgang.",
    );
  });

  test("Fassung B wählen aus dem Vergleich", async ({ page }) => {
    await setup(page);
    await tab(page, "Vergleich").click();
    await expect(page.getByTestId("compare-choose-a")).toBeDisabled();
    await page.getByTestId("compare-choose-b").click();
    await expect(page.getByTestId("compare-notice")).toContainText(
      "Fassung v2 ist jetzt aktiv.",
    );
    const activated = await calls(page, "transcript_variant_activate");
    expect(activated[0].args).toEqual({ variantId: "vb" });
    await expect(chip(page)).toContainText("Fassung v2");
    await expect(page.getByTestId("transcript-scroll")).toContainText(
      "von Stadtwerken.",
    );
  });

  test("Zusammenführen erzeugt eine dritte Fassung", async ({ page }) => {
    await setup(page);
    await tab(page, "Vergleich").click();
    await page.getByTestId("compare-merge").click();
    await expect(page.getByTestId("compare-notice")).toContainText(
      "Dritte Fassung v3 angelegt. Sie ist noch nicht aktiv.",
    );
    const merged = await calls(page, "transcript_variants_merge");
    expect(merged).toHaveLength(1);
    expect(merged[0].args).toEqual({
      meetingId: "y1",
      baseId: "va",
      otherId: "vb",
    });
    // Die dritte steht in der Liste, die aktive bleibt v1.
    await expect(chip(page)).toContainText("Fassung v1");
    await chip(page).click();
    await expect(
      menuItem(page, "v3 · Zusammengeführt (de) · 2 Segmente"),
    ).toBeEnabled();
  });

  test("eine verworfene Zusammenführung meldet es und legt nichts an", async ({
    page,
  }) => {
    await setup(page, { rejectMerge: true });
    await tab(page, "Vergleich").click();
    await page.getByTestId("compare-merge").click();
    await expect(page.getByTestId("compare-error")).toContainText(
      "Die Zusammenführung wurde verworfen",
    );
    await chip(page).click();
    await expect(page.getByRole("menuitem", { name: /^v3 · / })).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// Herkunft
// ---------------------------------------------------------------------------

const openOrigin = async (page: Page, area: string) => {
  await page
    .getByTestId(area)
    .click({ button: "right", position: { x: 12, y: 12 } });
  await clickItem(page, "Herkunft");
  const dialog = page.getByTestId("provenance-dialog");
  await expect(dialog).toBeVisible();
  return dialog;
};

test.describe("Herkunft per Rechtsklick", () => {
  test("am Transkript: Fassung, Quelle, Zeitpunkt, Auslöser", async ({
    page,
  }) => {
    await setup(page);
    const dialog = await openOrigin(page, "prov-area-transcript");
    await expect(dialog.getByTestId("prov-operation")).toContainText(
      "subtitles_import",
    );
    await expect(dialog.getByTestId("prov-sources")).toContainText("German");
    await expect(dialog.getByTestId("prov-sources")).toContainText("subtitle");
    await expect(dialog.getByTestId("prov-trigger")).toContainText("Nutzer");
    await expect(dialog.getByTestId("prov-time")).toContainText("2026");
    // Kein Modell bei Untertiteln, keine Konfidenz.
    await expect(dialog.getByTestId("prov-model")).toContainText("unbekannt");
    await expect(dialog.getByTestId("prov-confidence")).toHaveCount(0);
    const asked = await calls(page, "provenance_get");
    expect(asked[0].args).toEqual({
      contentType: "transcript_variant",
      id: "va",
    });
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  });

  test("an den KI-Notizen: Modell, Token, Dauer, Konfidenz", async ({
    page,
  }) => {
    await setup(page);
    await tab(page, "KI-Notizen").click();
    const dialog = await openOrigin(page, "prov-area-ai");
    await expect(dialog.getByTestId("prov-model")).toContainText("Gemma 4 E4B");
    await expect(dialog.getByTestId("prov-model")).toContainText("lokal");
    await expect(dialog.getByTestId("prov-tokens")).toContainText(
      "1200 ein · 340 aus",
    );
    await expect(dialog.getByTestId("prov-duration")).toContainText("18,4 s");
    await expect(dialog.getByTestId("prov-confidence")).toContainText("82 %");
    await expect(dialog.getByTestId("prov-sources")).toContainText(
      "Transkript v1",
    );
    await expect(dialog.getByTestId("prov-trigger")).toContainText("Nutzer");
    const asked = await calls(page, "provenance_get");
    expect(asked.at(-1)?.args).toEqual({ contentType: "document", id: "d-ai" });
  });

  test("am Protokoll mit der Zusammenfassung", async ({ page }) => {
    await setup(page);
    await tab(page, "Protokoll").click();
    await expect(page.getByTestId("prov-area-minutes")).toContainText(
      "Ein Video über den Lastgang.",
    );
    const dialog = await openOrigin(page, "prov-area-minutes");
    await expect(dialog.getByTestId("prov-operation")).toContainText("minutes");
    await expect(dialog.getByTestId("prov-model")).toContainText("Gemma 4 E4B");
    await expect(dialog.getByTestId("prov-tokens")).toContainText("340 aus");
    // Konfidenz nur, wenn es eine gibt.
    await expect(dialog.getByTestId("prov-confidence")).toHaveCount(0);
  });

  test("ohne Angaben sagt der Dialog es", async ({ page }) => {
    await setup(page, { single: true });
    await page.evaluate(() => {
      (window as any).__provenance = {};
    });
    const dialog = await openOrigin(page, "prov-area-transcript");
    await expect(dialog.getByTestId("provenance-empty")).toContainText(
      "keine Herkunftsangaben",
    );
  });
});

// ---------------------------------------------------------------------------
// yt-dlp: Untertitel und eigene Transkription
// ---------------------------------------------------------------------------

const HINT =
  "Für Untertitel und Transkription yt-dlp selbst installieren und in den Einstellungen „privat“ einschalten.";

test.describe("Untertitel und Transkription", () => {
  test("ohne Schalter oder ohne yt-dlp steht der Hinweis, der Player bleibt", async ({
    page,
  }) => {
    await setup(page, { privateOn: false });
    await expect(page.getByTestId("yt-tool-hint")).toHaveText(HINT);
    await expect(page.getByTestId("yt-subtitles")).toHaveCount(0);
    await expect(page.getByTestId("yt-own-transcription")).toHaveCount(0);
    await expect(page.getByTestId("yt-panel")).toBeVisible();
    await expect(page.getByTestId("yt-play")).toBeVisible();
    expect(await calls(page, "youtube_tool_detect")).toHaveLength(0);
  });

  test("Schalter an, aber yt-dlp fehlt: derselbe Hinweis", async ({ page }) => {
    await setup(page, { toolFound: false });
    await expect(page.getByTestId("yt-tool-hint")).toHaveText(HINT);
    await expect(page.getByTestId("yt-subtitles")).toHaveCount(0);
  });

  test("eine Untertitelspur wird sofort als Fassung geladen", async ({
    page,
  }) => {
    await setup(page);
    await page.getByTestId("yt-subtitles").click();
    await expect(page.getByTestId("yt-message")).toContainText(
      "Untertitel „German“ sind als Fassung angelegt.",
    );
    const fetched = await calls(page, "youtube_subtitles_fetch");
    expect(fetched).toHaveLength(1);
    expect(fetched[0].args.track.language).toBe("de");
    await chip(page).click();
    await expect(
      menuItem(page, /^v3 · Untertitel \(manuell\) \(de\)/),
    ).toBeVisible();
  });

  test("bei mehreren Spuren wählt der Nutzer die Sprache", async ({ page }) => {
    await setup(page, {
      tracks: [
        { language: "de", name: "German", auto: false },
        { language: "en-orig", name: "English (Original)", auto: true },
      ],
    });
    await page.getByTestId("yt-subtitles").click();
    await expect(page.getByTestId("yt-track-pick")).toBeVisible();
    expect(await calls(page, "youtube_subtitles_fetch")).toHaveLength(0);
    await page.getByTestId("yt-track-select").click();
    await page
      .getByRole("option", {
        name: "English (Original) [en-orig] (automatisch)",
      })
      .click();
    await page.getByTestId("yt-track-load").click();
    const fetched = await calls(page, "youtube_subtitles_fetch");
    expect(fetched[0].args.track).toMatchObject({
      language: "en-orig",
      auto: true,
    });
    await chip(page).click();
    await expect(
      menuItem(page, /^v3 · Untertitel \(automatisch\) \(en-orig\)/),
    ).toBeVisible();
  });

  test("ohne Untertitel bleibt nur die eigene Transkription, mit Hinweis", async ({
    page,
  }) => {
    await setup(page, { tracks: [] });
    await page.getByTestId("yt-subtitles").click();
    await expect(page.getByTestId("yt-message")).toHaveText(
      "Dieses Video hat keine Untertitel. Es bleibt die eigene Transkription.",
    );
    expect(await calls(page, "youtube_subtitles_fetch")).toHaveLength(0);
    await page.getByTestId("yt-own-transcription").click();
    await expect(page.getByTestId("yt-message")).toContainText(
      "Die eigene Transkription ist fertig.",
    );
    const own = await calls(page, "youtube_own_transcription");
    expect(own).toHaveLength(1);
    expect(own[0].args).toEqual({ meetingId: "y1", modelId: null });
  });

  test("ein Fehler von yt-dlp wird verständlich gemeldet", async ({ page }) => {
    await setup(page);
    await page.evaluate(() => {
      const w = window as any;
      const inner = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (cmd: string, args: any) => {
        if (cmd === "youtube_subtitle_tracks") throw "youtube_tool_outdated";
        return inner(cmd, args);
      };
    });
    await page.getByTestId("yt-subtitles").click();
    await expect(page.getByTestId("yt-message")).toContainText(
      "yt-dlp ist veraltet – bitte aktualisieren",
    );
  });
});

// ---------------------------------------------------------------------------
// Dauer aus dem Player
// ---------------------------------------------------------------------------

test("die Dauer wird aus dem Player nachgetragen, wenn sie fehlt", async ({
  page,
}) => {
  await setup(page);
  expect(await calls(page, "youtube_set_duration")).toHaveLength(0);
  await page.getByTestId("yt-play").click();
  await expect(page.getByTestId("yt-iframe")).toBeVisible();
  // Der Player meldet „spielt“: erst dann kennt er die Dauer.
  await page.evaluate(() =>
    (window as any).__yt.players[0].opts.events.onStateChange({ data: 1 }),
  );
  await expect
    .poll(async () => (await calls(page, "youtube_set_duration")).length)
    .toBe(1);
  const set = await calls(page, "youtube_set_duration");
  expect(set[0].args).toEqual({ meetingId: "y1", seconds: 1234.5 });
  // Ein zweites „spielt“ trägt nichts noch einmal nach.
  await page.evaluate(() =>
    (window as any).__yt.players[0].opts.events.onStateChange({ data: 1 }),
  );
  expect(await calls(page, "youtube_set_duration")).toHaveLength(1);
});

// ---------------------------------------------------------------------------
// Barrierefreiheit der neuen Teile (axe, ohne den app-weiten Kontrastbefund)
// ---------------------------------------------------------------------------

const axeSevere = async (page: Page) =>
  (
    await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"])
      .disableRules(["color-contrast"])
      .analyze()
  ).violations
    .filter((v) => v.impact === "critical" || v.impact === "serious")
    .map(
      (v) => `${v.id}: ${v.nodes.map((n) => n.target.join(" ")).join(" | ")}`,
    );

test.describe("axe", () => {
  test("Fassungs-Chip, Vergleich und Werkzeuge ohne schwere Befunde", async ({
    page,
  }) => {
    await setup(page);
    expect(await axeSevere(page), "Transkript mit Chip und Werkzeugen").toEqual(
      [],
    );
    await tab(page, "Vergleich").click();
    await expect(page.getByTestId("diff-row")).toHaveCount(1);
    expect(await axeSevere(page), "Vergleich").toEqual([]);
  });

  test("der Herkunft-Dialog ohne schwere Befunde", async ({ page }) => {
    await setup(page);
    await tab(page, "KI-Notizen").click();
    await openOrigin(page, "prov-area-ai");
    expect(await axeSevere(page), "Herkunft").toEqual([]);
  });
});
