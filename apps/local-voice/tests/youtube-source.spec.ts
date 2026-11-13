import { test, expect, type Page } from "@playwright/test";
import {
  calls,
  emitMeeting,
  goToRecordings,
  installRecMock,
  openRecordings,
  pickMeeting,
} from "./recLayoutMock";
import { installTauriMock } from "./calendarMock";
import {
  captureSegment,
  installYoutubeMock,
  pasteText,
  playerCalls,
  playerCount,
  YT_CHANNEL,
  YT_ID,
  YT_MEETING_ID,
  YT_TITLE,
  type YoutubeMockOptions,
} from "./youtubeMock";
import { geheZu } from "./profilmenue";

// A2 (#66, AK2/AK3): YouTube als Quelle gegen die Tauri-Attrappe. Link einfuegen
// (Dialog und Strg+V), Meldungen fuer ungueltige und Playlist-Links, der Player
// in der Inhaltsspalte (einklappbar, Hoehe, erst nach Klick), Zeitsprung beim Klick
// auf ein Transkript-Segment, die Einstellung "privat" mit yt-dlp-Erkennung.
// #537: "Beim Abspielen mitschreiben" (Einwilligung, Start, Hinweise, Stopp).
// Die Link-Logik selbst pruefen die Rust-Tests (`cargo test --lib youtube::`).

test.use({ timezoneId: "Europe/Berlin", locale: "de-DE" });

const LINK = `https://youtu.be/${YT_ID}?si=TRACK&t=1m30s`;

const setup = async (
  page: Page,
  options: YoutubeMockOptions & { recording?: boolean } = {},
) => {
  await installRecMock(page, { recording: options.recording });
  const mock = await installYoutubeMock(page, options);
  await openRecordings(page, 1366, 768);
  return mock;
};

const dialogOf = (page: Page) =>
  page.getByRole("dialog", { name: "YouTube-Link einfügen" });

const openDialog = async (page: Page) => {
  await page.getByTestId("link-open").click();
  const dialog = dialogOf(page);
  await expect(dialog).toBeVisible();
  return dialog;
};

// ---------------------------------------------------------------------------
// AK2: Link einfuegen
// ---------------------------------------------------------------------------

test.describe("Link einfügen", () => {
  test("erzeugt eine Besprechung mit Quelle YouTube, Titel und Kanal", async ({
    page,
  }) => {
    const { youtubeRequests } = await setup(page, { existing: false });
    // Der Knopf ist aktiv (vorher ausgegraut, "folgt mit #65").
    await expect(page.getByTestId("link-open")).toBeEnabled();
    const dialog = await openDialog(page);
    await expect(dialog.getByTestId("yt-add")).toBeDisabled();
    await dialog.getByTestId("yt-link-input").fill(LINK);
    await expect(dialog.getByTestId("yt-link-check")).toContainText(
      "Video erkannt",
    );
    await expect(dialog.getByTestId("yt-link-check")).toContainText(
      "Start bei 1:30",
    );
    // Nichts angelegt, bevor der Nutzer bestaetigt.
    expect(await calls(page, "youtube_add_source")).toHaveLength(0);

    await dialog.getByTestId("yt-add").click();
    await expect(dialog).toHaveCount(0);

    const added = await calls(page, "youtube_add_source");
    expect(added).toHaveLength(1);
    expect(added[0].args).toEqual({
      url: LINK,
      projectId: null,
      targetMeetingId: null,
    });

    // Die neue Besprechung ist gewaehlt: Titel, Quelle YouTube, Kanal.
    await expect(page.getByTestId("meeting-title")).toHaveText(YT_TITLE);
    await expect(page.getByTestId("rec-detail-head")).toContainText("YouTube");
    await expect(page.getByTestId("yt-panel")).toBeVisible();
    await expect(page.getByTestId("yt-channel")).toHaveText(YT_CHANNEL);
    // Sie steht in der Liste.
    await expect(page.locator('[data-meeting-id="y-neu"]')).toContainText(
      YT_TITLE,
    );
    // Datenschutz: das Anlegen fragt das Backend, die Oberflaeche verbindet sich
    // nicht mit YouTube.
    expect(youtubeRequests).toEqual([]);
  });

  test("youtu.be, shorts und embed gehen denselben Weg", async ({ page }) => {
    await setup(page, { existing: false });
    for (const url of [
      `https://www.youtube.com/watch?v=${YT_ID}`,
      `https://youtu.be/${YT_ID}`,
      `https://www.youtube.com/shorts/${YT_ID}`,
      `https://www.youtube.com/embed/${YT_ID}`,
      `https://m.youtube.com/watch?v=${YT_ID}&feature=share`,
    ]) {
      const dialog = await openDialog(page);
      await dialog.getByTestId("yt-link-input").fill(url);
      await expect(dialog.getByTestId("yt-link-check")).toHaveAttribute(
        "data-state",
        "ok",
      );
      await page.keyboard.press("Escape");
      await expect(dialog).toHaveCount(0);
    }
  });

  test("das gewählte Projekt ist vorbelegt, eine andere Wahl wird übergeben", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem("lva.ui.meetings.project", "f2");
    });
    await setup(page, { existing: false });
    const dialog = await openDialog(page);
    await expect(dialog.getByTestId("yt-link-project")).toContainText(
      "Geschäftlich",
    );
    await dialog.getByTestId("yt-link-input").fill(`https://youtu.be/${YT_ID}`);
    await dialog.getByTestId("yt-add").click();
    await expect(dialog).toHaveCount(0);
    expect((await calls(page, "youtube_add_source"))[0].args).toEqual({
      url: `https://youtu.be/${YT_ID}`,
      projectId: "f2",
      targetMeetingId: null,
    });

    // Eine andere Wahl im Dialog gilt.
    const again = await openDialog(page);
    await again.getByTestId("yt-link-input").fill(`https://youtu.be/${YT_ID}`);
    await again
      .getByTestId("yt-link-project")
      .locator(".app-select__control")
      .click();
    await page.getByRole("option", { name: "Privat", exact: true }).click();
    await again.getByTestId("yt-add").click();
    await expect(again).toHaveCount(0);
    const all = await calls(page, "youtube_add_source");
    expect(all).toHaveLength(2);
    expect(all[1].args).toEqual({
      url: `https://youtu.be/${YT_ID}`,
      projectId: "f1",
      targetMeetingId: null,
    });
  });

  test("ungültige, fremde und Playlist-Links: verständliche Meldung, Hinzufügen gesperrt", async ({
    page,
  }) => {
    await setup(page, { existing: false });
    const dialog = await openDialog(page);
    const input = dialog.getByTestId("yt-link-input");
    const check = dialog.getByTestId("yt-link-check");
    const cases: [string, string][] = [
      [
        "https://www.youtube.com/playlist?list=PLabcdefghij",
        "Playlists folgen später",
      ],
      [
        `https://www.youtube.com/watch?list=PLabcdefghij`,
        "Playlists folgen später",
      ],
      ["https://www.youtube.com/@wolffappliedai", "Kanäle werden nicht unterstützt"],
      ["https://vimeo.com/123456789", "Das ist kein YouTube-Link"],
      ["https://www.youtube.com/watch?v=zukurz", "kein gültiger YouTube-Link"],
      ["hallo welt", "kein gültiger YouTube-Link"],
    ];
    for (const [url, message] of cases) {
      await input.fill(url);
      await expect(check).toHaveAttribute("data-state", "error");
      await expect(check).toContainText(message);
      await expect(dialog.getByTestId("yt-add")).toBeDisabled();
    }
    // Enter legt auch nichts an, solange der Link nicht gilt.
    await input.press("Enter");
    expect(await calls(page, "youtube_add_source")).toHaveLength(0);
    // Ein guter Link hebt die Sperre auf.
    await input.fill(`https://youtu.be/${YT_ID}`);
    await expect(dialog.getByTestId("yt-add")).toBeEnabled();
  });

  test("ein Fehler des Backends bleibt im Dialog sichtbar, es entsteht nichts", async ({
    page,
  }) => {
    await setup(page, { existing: false });
    const dialog = await openDialog(page);
    await dialog.getByTestId("yt-link-input").fill("https://youtu.be/unavailable");
    await dialog.getByTestId("yt-add").click();
    await expect(dialog.getByTestId("yt-link-error")).toContainText(
      "nicht verfügbar",
    );
    await expect(dialog).toBeVisible();
    await expect(page.locator('[data-meeting-id="y-neu"]')).toHaveCount(0);
  });

  test("Strg+V mit einem YouTube-Link öffnet den Dialog, anderer Text nicht", async ({
    page,
  }) => {
    await setup(page, { existing: false });
    // Anderer Text: nichts geschieht, das Einfuegen bleibt unberuehrt.
    expect(await pasteText(page, "Protokoll vom 28.09.: YouTube später")).toBe(
      false,
    );
    expect(await pasteText(page, "https://example.com/")).toBe(false);
    await expect(dialogOf(page)).toHaveCount(0);

    expect(await pasteText(page, `  ${LINK}  `)).toBe(true);
    const dialog = dialogOf(page);
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("yt-link-input")).toHaveValue(LINK);
    await expect(dialog.getByTestId("yt-link-check")).toContainText(
      "Video erkannt",
    );
    // Im offenen Dialog fuegt das Eingabefeld selbst ein, nicht die Seite.
    expect(await pasteText(page, LINK)).toBe(false);
  });

  test("Strg+V in ein Eingabefeld gehört dem Feld", async ({ page }) => {
    await setup(page, { existing: false });
    // Das Suchfeld der Liste ist ein Eingabefeld.
    const field = page.getByRole("searchbox").first();
    await field.focus();
    const intercepted = await page.evaluate((text) => {
      const data = new DataTransfer();
      data.setData("text/plain", text);
      const event = new ClipboardEvent("paste", {
        clipboardData: data,
        bubbles: true,
        cancelable: true,
      });
      document.activeElement!.dispatchEvent(event);
      return event.defaultPrevented;
    }, LINK);
    expect(intercepted).toBe(false);
    await expect(dialogOf(page)).toHaveCount(0);
  });
});

// ---------------------------------------------------------------------------
// AK3: Player in der Inhaltsspalte
// ---------------------------------------------------------------------------

test.describe("Player", () => {
  test("der Bereich steht oben in der Inhaltsspalte, nichts verbindet sich vor dem Klick", async ({
    page,
  }) => {
    const { youtubeRequests } = await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    const panel = page.getByTestId("yt-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toHaveAttribute("data-phase", "idle");
    await expect(page.getByTestId("yt-channel")).toHaveText(YT_CHANNEL);
    // Er liegt in der Inhaltsspalte und VOR dem Kopf der Besprechung.
    const content = page.getByTestId("rec-content");
    await expect(content.getByTestId("yt-panel")).toBeVisible();
    const panelBox = (await panel.boundingBox())!;
    const headBox = (await page.getByTestId("rec-detail-head").boundingBox())!;
    expect(panelBox.y).toBeLessThan(headBox.y);
    // 16:9-Flaeche mit Hoehe 270 px, kein Rahmen, kein Skript, keine Anfrage.
    await expect(page.getByTestId("yt-stage")).toHaveCSS("height", "270px");
    await expect(page.getByTestId("yt-iframe")).toHaveCount(0);
    await expect(page.getByTestId("yt-play")).toBeVisible();
    expect(youtubeRequests).toEqual([]);
    expect(await playerCount(page)).toBe(0);
    // Die Seite scrollt nicht durch den Player (eine Scrollbar je Bereich).
    const scrolls = await page.evaluate(
      () => document.scrollingElement!.scrollHeight <= innerHeight,
    );
    expect(scrolls).toBe(true);
  });

  test("Video abspielen lädt den offiziellen Einbett-Player (youtube-nocookie, IFrame API)", async ({
    page,
  }) => {
    const { youtubeRequests } = await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-play").click();
    const frame = page.getByTestId("yt-iframe");
    await expect(frame).toBeVisible();
    const src = (await frame.getAttribute("src"))!;
    const url = new URL(src);
    expect(url.origin).toBe("https://www.youtube-nocookie.com");
    expect(url.pathname).toBe(`/embed/${YT_ID}`);
    expect(url.searchParams.get("enablejsapi")).toBe("1");
    expect(url.searchParams.get("origin")).toBe(new URL(page.url()).origin);
    expect(url.searchParams.get("rel")).toBe("0");
    await expect(frame).toHaveAttribute(
      "referrerpolicy",
      "strict-origin-when-cross-origin",
    );
    // Ein Rahmen, unmittelbar im Bereich: kein verschachteltes iframe.
    await expect(page.getByTestId("yt-stage").locator("iframe")).toHaveCount(1);
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    expect(await playerCount(page)).toBe(1);
    expect(await playerCalls(page)).toEqual([["playVideo"]]);
    // Der einzige Verkehr zu YouTube ist der Einbett-Rahmen.
    expect(youtubeRequests).toHaveLength(1);
    expect(youtubeRequests[0]).toMatch(
      /^https:\/\/www\.youtube-nocookie\.com\/embed\//,
    );
  });

  test("Einklappen pausiert und bleibt nach Neuladen; Griff ändert die Höhe und bleibt", async ({
    page,
  }) => {
    await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    const panel = page.getByTestId("yt-panel");
    const stage = page.getByTestId("yt-stage");
    await page.getByTestId("yt-play").click();
    await expect(panel).toHaveAttribute("data-phase", "ready");

    // Hoehe per Tastatur am Griff: Pfeil runter = +16 px, Pos1/Ende = Grenzen.
    const handle = page.getByTestId("yt-resize");
    await handle.focus();
    await handle.press("ArrowDown");
    await expect(stage).toHaveCSS("height", "286px");
    await handle.press("ArrowUp");
    await handle.press("ArrowUp");
    await expect(stage).toHaveCSS("height", "254px");
    await handle.press("Home");
    await expect(stage).toHaveCSS("height", "120px");
    await handle.press("End");
    await expect(stage).toHaveCSS("height", "560px");
    await handle.dblclick();
    await expect(stage).toHaveCSS("height", "270px");
    await handle.press("ArrowDown");
    await expect(stage).toHaveAttribute("style", /height: 286px/);

    // Ziehen mit der Maus.
    const box = (await handle.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 34, {
      steps: 4,
    });
    await page.mouse.up();
    await expect(stage).toHaveCSS("height", "320px");

    // Einklappen: das Video stoppt, der Bereich verschwindet.
    await page.getByTestId("yt-toggle").click();
    await expect(panel).toHaveAttribute("data-open", "false");
    await expect(stage).toBeHidden();
    expect((await playerCalls(page)).at(-1)).toEqual(["pauseVideo"]);
    await expect(page.getByTestId("yt-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    // Beides uebersteht das Neuladen.
    await page.reload();
    await goToRecordings(page);
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-open",
      "false",
    );
    await page.getByTestId("yt-toggle").click();
    await expect(page.getByTestId("yt-stage")).toHaveCSS("height", "320px");
    // Nach dem Neuladen laedt auch nichts von selbst.
    await expect(page.getByTestId("yt-iframe")).toHaveCount(0);
  });

  test("Auf YouTube öffnen nutzt die bereinigte Adresse", async ({ page }) => {
    await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-open-external").click();
    await expect
      .poll(async () => (await calls(page, "plugin:opener|open_url")).length)
      .toBe(1);
    expect((await calls(page, "plugin:opener|open_url"))[0].args.url).toBe(
      `https://www.youtube.com/watch?v=${YT_ID}`,
    );
  });

  test("ein Klick auf ein Transkript-Segment ruft seekTo des Players", async ({
    page,
  }) => {
    await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    const first = page.locator(
      '[data-segment-index="0"] [data-act="seek"]',
    );
    // Die Zeitmarken sind Knoepfe, sobald ein Player da ist (vorher reiner Text).
    await expect(first).toBeVisible();
    await expect(first).toHaveAttribute("data-seek-ms", "4000");
    await expect(page.getByTestId("yt-iframe")).toHaveCount(0);

    // Der Klick laedt den Player und springt dann (ein bewusster Klick).
    await first.click();
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    expect(await playerCalls(page)).toEqual([
      ["seekTo", 4, true],
      ["playVideo"],
    ]);

    // Jeder weitere Klick springt direkt.
    await page
      .locator('[data-segment-index="3"] [data-act="seek"]')
      .click();
    await page
      .locator('[data-segment-index="10"] [data-act="seek"]')
      .click();
    const calls3 = await playerCalls(page);
    expect(calls3.slice(2)).toEqual([
      ["seekTo", 91, true],
      ["playVideo"],
      ["seekTo", 294, true],
      ["playVideo"],
    ]);
    expect(await playerCount(page)).toBe(1);
  });

  test("ein Klick auf eine Zeitmarke klappt den eingeklappten Player auf", async ({
    page,
  }) => {
    await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-toggle").click();
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-open",
      "false",
    );
    await page.locator('[data-segment-index="2"] [data-act="seek"]').click();
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-open",
      "true",
    );
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    expect(await playerCalls(page)).toEqual([
      ["seekTo", 62, true],
      ["playVideo"],
    ]);
  });

  test("eine Audio-Besprechung bleibt unverändert: kein Player-Bereich, Zeitmarken spielen das Audio", async ({
    page,
  }) => {
    await setup(page);
    await pickMeeting(page, "m2");
    await expect(page.getByTestId("rec-player")).toBeVisible();
    await expect(page.getByTestId("yt-panel")).toHaveCount(0);
    expect(await calls(page, "youtube_source_get")).toHaveLength(0);
  });

  test("ein Fehler des Players (Einbetten verboten) wird erklärt, Erneut versuchen baut neu auf", async ({
    page,
  }) => {
    await setup(page);
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-play").click();
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    await page.evaluate(() => {
      const player = (window as any).__yt.players.at(-1);
      player.opts.events.onError({ data: 101 });
    });
    await expect(page.getByTestId("yt-error")).toContainText(
      "außerhalb von YouTube",
    );
    await expect(page.getByTestId("yt-open-external")).toBeVisible();
    await page.getByTestId("yt-retry").click();
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    expect(await playerCount(page)).toBe(2);
  });

  test("lässt sich die Player-API nicht laden, steht eine Meldung da statt eines leeren Rahmens", async ({
    page,
  }) => {
    await installRecMock(page);
    const { youtubeRequests } = await installYoutubeMock(page, {
      fakePlayer: false,
    });
    await page.route("**/iframe_api", (route) => route.abort());
    await openRecordings(page, 1366, 768);
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-play").click();
    await expect(page.getByTestId("yt-error")).toContainText(
      "konnte nicht geladen werden",
    );
    expect(await playerCount(page)).toBe(0);
    await expect(page.getByTestId("yt-retry")).toBeVisible();
    // Das Skript kommt von der offiziellen Adresse und nur auf Klick.
    expect(youtubeRequests.every((u) => u.startsWith("https://www.youtube"))).toBe(
      true,
    );
  });
});

// ---------------------------------------------------------------------------
// Einstellung "privat" und yt-dlp
// ---------------------------------------------------------------------------

test.describe("Einstellung privat", () => {
  const openSettings = async (page: Page) => {
    await installTauriMock(page, "main");
    await installYoutubeMock(page, { fakePlayer: false, existing: false });
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto("/");
    await geheZu(page, "Einstellungen");
    await expect(
      page.getByText("YouTube: selbst installiertes yt-dlp nutzen", {
        exact: false,
      }),
    ).toBeVisible();
  };

  const privateSwitch = (page: Page) =>
    page
      .getByText("YouTube: selbst installiertes yt-dlp nutzen (privat, experimentell)", {
        exact: true,
      })
      .locator("xpath=ancestor::div[.//input[@type='checkbox']][1]")
      .locator("input[type=checkbox]");

  test("Standard aus: kein Pfadfeld, keine Suche nach yt-dlp", async ({
    page,
  }) => {
    await openSettings(page);
    await expect(privateSwitch(page)).not.toBeChecked();
    await expect(page.getByTestId("yt-tool-settings")).toHaveCount(0);
    expect(await calls(page, "youtube_tool_detect")).toHaveLength(0);
    expect(
      await calls(page, "change_meeting_youtube_private_setting"),
    ).toHaveLength(0);
  });

  test("Einschalten zeigt Pfad, Version und den Hinweis zur Rechtslage", async ({
    page,
  }) => {
    await openSettings(page);
    await privateSwitch(page).evaluate((el) => (el as HTMLInputElement).click());
    await expect(page.getByTestId("yt-tool-settings")).toBeVisible();
    const last = (await calls(page, "change_meeting_youtube_private_setting")).at(-1);
    expect(last?.args).toEqual({ enabled: true });
    await expect(page.getByTestId("yt-tool-status")).toContainText(
      "yt-dlp gefunden: Version 2026.09.01",
    );
    await expect(page.getByTestId("yt-tool-status")).toHaveAttribute(
      "data-found",
      "true",
    );
    // Kurz und sachlich: Nutzungsbedingungen, Urheberrecht, Urteil, eigene Verantwortung.
    const legal = page.getByTestId("yt-tool-legal");
    await expect(legal).toContainText("Nutzungsbedingungen");
    await expect(legal).toContainText("5 U 54/23");
    await expect(legal).toContainText("eigene Verantwortung");
    await expect(legal).toContainText("Keine Rechtsberatung");
    expect((await legal.textContent())!.length).toBeLessThan(420);
  });

  test("nicht gefunden: eine verständliche Meldung, nichts wird geladen", async ({
    page,
  }) => {
    await openSettings(page);
    await page.evaluate(() => {
      (window as any).__toolStatus = {
        found: false,
        path: null,
        version: null,
        source: null,
        error: "not_found",
      };
    });
    await privateSwitch(page).evaluate((el) => (el as HTMLInputElement).click());
    await expect(page.getByTestId("yt-tool-status")).toContainText(
      "yt-dlp nicht gefunden",
    );
    await expect(page.getByTestId("yt-tool-status")).toHaveAttribute(
      "data-found",
      "false",
    );
  });

  test("ein eingetragener Pfad wird gespeichert und geprüft", async ({ page }) => {
    await openSettings(page);
    await privateSwitch(page).evaluate((el) => (el as HTMLInputElement).click());
    await expect(page.getByTestId("yt-tool-status")).toContainText("Version");
    const input = page.getByTestId("yt-tool-path");
    await input.fill("C:\\Tools\\yt-dlp.exe");
    await input.press("Enter");
    await expect
      .poll(
        async () =>
          (await calls(page, "change_meeting_youtube_tool_path_setting")).length,
      )
      .toBe(1);
    expect(
      (await calls(page, "change_meeting_youtube_tool_path_setting"))[0].args,
    ).toEqual({ path: "C:\\Tools\\yt-dlp.exe" });
    // Geprueft wird der Wert aus dem Feld.
    const detects = await calls(page, "youtube_tool_detect");
    expect(detects.at(-1)?.args).toEqual({ path: "C:\\Tools\\yt-dlp.exe" });
  });
});

// ---------------------------------------------------------------------------
// #537: Beim Abspielen mitschreiben (ohne yt-dlp, ohne Download)
// ---------------------------------------------------------------------------

test.describe("Beim Abspielen mitschreiben", () => {
  const START = "meetings_start_playback_capture";
  const NEED_TOOL =
    "Schneller mit Untertiteln: yt-dlp selbst installieren und unter Einstellungen „privat“ einschalten (optional).";

  const captureDialog = (page: Page) =>
    page.getByRole("dialog", { name: "Beim Abspielen mitschreiben" });

  /** Knopf, Einwilligung bestaetigen. */
  const startCapture = async (page: Page) => {
    await page.getByTestId("yt-capture").click();
    const dialog = captureDialog(page);
    await expect(dialog).toBeVisible();
    await dialog.getByTestId("yt-capture-confirm").click();
  };

  test("ohne yt-dlp ist Mitschreiben der erste Schritt, mit Erklärung und kleinem Hinweis", async ({
    page,
  }) => {
    const { youtubeRequests } = await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    const block = page.getByTestId("yt-capture-block");
    await expect(block).toBeVisible();
    await expect(page.getByTestId("yt-capture")).toHaveText(
      "Beim Abspielen mitschreiben",
    );
    await expect(page.getByTestId("yt-capture-explain")).toContainText(
      "Der Systemton wird beim Abspielen im eingebetteten Player in Echtzeit transkribiert.",
    );
    // Der Hinweis auf yt-dlp steht darunter, klein, und ist optional.
    await expect(page.getByTestId("yt-tool-hint")).toHaveText(NEED_TOOL);
    const blockBox = (await block.boundingBox())!;
    const hintBox = (await page.getByTestId("yt-tool-hint").boundingBox())!;
    expect(hintBox.y).toBeGreaterThan(blockBox.y);
    await expect(page.getByTestId("yt-subtitles")).toHaveCount(0);
    await expect(page.getByTestId("yt-own-transcription")).toHaveCount(0);
    // Der Leerzustand verweist auf den Knopf (keine Aufnahme laeuft).
    await expect(
      page.getByText("Noch kein Transkript. Starte oben „Beim Abspielen mitschreiben“."),
    ).toBeVisible();
    // Bis zur Bestaetigung passiert nichts: kein Start, kein Netz.
    expect(await calls(page, START)).toHaveLength(0);
    expect(youtubeRequests).toEqual([]);
  });

  test("mit yt-dlp bleiben Untertitel und eigene Transkription, dazu der Knopf zum Mitschreiben", async ({
    page,
  }) => {
    await setup(page, { empty: true, privateOn: true });
    await pickMeeting(page, YT_MEETING_ID);
    await expect(page.getByTestId("yt-subtitles")).toBeVisible();
    await expect(page.getByTestId("yt-own-transcription")).toBeVisible();
    await expect(page.getByTestId("yt-capture")).toBeVisible();
    // Der Hinweis auf yt-dlp entfaellt, wenn es da ist.
    await expect(page.getByTestId("yt-tool-hint")).toHaveCount(0);
    await expect(page.getByTestId("yt-capture-block")).toHaveCount(0);
  });

  test("mit vorhandenem Transkript wird das Mitschreiben nicht angeboten", async ({
    page,
  }) => {
    await setup(page, { empty: false });
    await pickMeeting(page, YT_MEETING_ID);
    await expect(page.getByTestId("yt-tool-hint")).toBeVisible();
    await expect(page.getByTestId("yt-capture")).toHaveCount(0);
    await expect(page.getByTestId("yt-capture-block")).toHaveCount(0);
  });

  test("Abbrechen im Einwilligungsdialog startet nichts", async ({ page }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-capture").click();
    const dialog = captureDialog(page);
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("yt-capture-dialog")).toContainText(
      "Einwilligung erforderlich",
    );
    await expect(dialog.getByTestId("yt-capture-dialog")).toContainText(
      "nur den Systemton, nicht das Mikrofon",
    );
    await dialog.getByRole("button", { name: "Abbrechen" }).last().click();
    await expect(dialog).toHaveCount(0);
    expect(await calls(page, START)).toHaveLength(0);
    await expect(page.getByTestId("yt-capture")).toBeVisible();
  });

  test("Bestätigen startet das Mitschreiben in genau diesen Eintrag und spielt das Video von vorn", async ({
    page,
  }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await expect.poll(async () => (await calls(page, START)).length).toBe(1);
    // Mit einer Vorgangs-Id (lva#538): ueber sie meint ein Abbruch genau diesen Start.
    expect((await calls(page, START))[0].args).toEqual({
      meetingId: YT_MEETING_ID,
      consentConfirmed: true,
      opId: expect.stringMatching(/^.{8,}$/),
    });
    // Der Dialog geht zu, der Knopf weicht dem Hinweis "es wird mitgeschrieben".
    await expect(captureDialog(page)).toHaveCount(0);
    await expect(page.getByTestId("yt-capture-running")).toContainText(
      "Es wird mitgeschrieben",
    );
    await expect(page.getByTestId("yt-capture")).toHaveCount(0);
    // Der offizielle Player startet von vorn; kein Hinweis "Jetzt abspielen" noetig.
    await expect(page.getByTestId("yt-panel")).toHaveAttribute(
      "data-phase",
      "ready",
    );
    expect(await playerCalls(page)).toEqual([["seekTo", 0, true], ["playVideo"]]);
    await expect(page.getByTestId("yt-capture-play-hint")).toHaveCount(0);
    // Die normale Aufnahmesteuerung ist da, der Leerzustand der Aufnahme auch.
    await expect(page.getByTestId("rec-live")).toBeVisible();
    await expect(page.getByText("Noch keine Segmente", { exact: false })).toBeVisible();
    await expect(
      page.getByText("Noch kein Transkript. Starte oben", { exact: false }),
    ).toHaveCount(0);
  });

  test("Sätze des Systemtons erscheinen live, Beenden ist der normale Stopp-Knopf", async ({
    page,
  }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await expect(page.getByTestId("yt-capture-running")).toBeVisible();
    await captureSegment(page, 0, "Willkommen zum Vortrag über Lastgänge.");
    const row = page.locator('[data-segment-index="0"]');
    await expect(row).toContainText("Willkommen zum Vortrag über Lastgänge.");
    // Ein Kanal allein bekommt kein Sprecherschild, schon gar keine "Gegenseite".
    await expect(row).not.toContainText("Gegenseite");
    await page.getByTestId("rec-stop").click();
    await expect.poll(async () => (await calls(page, "meetings_stop")).length).toBe(1);
  });

  test("kommt kein Ton an, fragt die Oberfläche nach und nimmt es mit dem Ton zurück", async ({
    page,
  }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await expect(page.getByTestId("yt-capture-running")).toBeVisible();
    await expect(page.getByTestId("yt-capture-quiet")).toHaveCount(0);
    // Ein stiller Mikrofonkanal (0) ist hier kein Thema, nur der Systemton (1).
    await emitMeeting(page, {
      kind: "health",
      meeting_id: YT_MEETING_ID,
      channel: 0,
      state: "silent",
    });
    await expect(page.getByTestId("yt-capture-quiet")).toHaveCount(0);
    await emitMeeting(page, {
      kind: "health",
      meeting_id: YT_MEETING_ID,
      channel: 1,
      state: "silent",
    });
    await expect(page.getByTestId("yt-capture-quiet")).toContainText(
      "kommt kein Ton an",
    );
    await emitMeeting(page, {
      kind: "health",
      meeting_id: YT_MEETING_ID,
      channel: 1,
      state: "recovered",
    });
    await expect(page.getByTestId("yt-capture-quiet")).toHaveCount(0);
  });

  test("scheitert der Systemton, steht eine verständliche Meldung, der Eintrag bleibt wie er war", async ({
    page,
  }) => {
    await setup(page, { empty: true, startError: "playback_capture_failed: kein Ausgabegerät" });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await expect(page.getByTestId("yt-message")).toContainText(
      "Der Systemton ließ sich nicht aufnehmen.",
    );
    await expect(captureDialog(page)).toHaveCount(0);
    // Keine Aufnahme laeuft; der Knopf ist wieder da, und der Player blieb unberuehrt.
    await expect(page.getByTestId("yt-capture-running")).toHaveCount(0);
    await expect(page.getByTestId("yt-capture")).toBeEnabled();
    expect(await playerCount(page)).toBe(0);
    await expect(
      page.getByText("Noch kein Transkript. Starte oben", { exact: false }),
    ).toBeVisible();
  });

  test("ein Eintrag, der nicht leer ist, wird abgewiesen (target_not_empty)", async ({
    page,
  }) => {
    await setup(page, { empty: true, startError: "target_not_empty" });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await expect(page.getByTestId("yt-message")).toContainText(
      "Dieser Eintrag hat schon ein Transkript oder eine Aufnahme.",
    );
    await expect(page.getByTestId("yt-capture-running")).toHaveCount(0);
  });

  // lva#538: Einwilligungstext praezise (Berechtigung unbeschraenkt, Urheberrecht und
  // Plattformbedingungen, der gesamte Systemausgang wird erfasst).
  test("der Einwilligungstext verlangt die Berechtigung uneingeschränkt und warnt vor dem gesamten Systemausgang", async ({
    page,
  }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await page.getByTestId("yt-capture").click();
    const body = captureDialog(page).getByTestId("yt-capture-dialog");
    await expect(body).toContainText("erforderliche Berechtigung");
    await expect(body).toContainText("Urheberrecht");
    await expect(body).toContainText("Nutzungsbedingungen der Plattform");
    await expect(body).toContainText(
      "Dass das Video öffentlich zugänglich ist, reicht dafür nicht aus.",
    );
    // Der gesamte Ton des Ausgabegeraets, auch Benachrichtigungen und Anrufe.
    await expect(body).toContainText("gesamte Ton des gewählten Ausgabegeräts");
    await expect(body).toContainText("Benachrichtigungstöne, Anrufe");
    // Die alte Formulierung, die oeffentliche Verfuegbarkeit als Erlaubnis darstellt:
    await expect(body).not.toContainText(
      "öffentlich zugänglich, oder alle darin zu hörenden Personen",
    );
  });

  test("die Stillewarnung nennt auch den falschen Audioausgang", async ({
    page,
  }) => {
    await setup(page, { empty: true });
    await pickMeeting(page, YT_MEETING_ID);
    await startCapture(page);
    await emitMeeting(page, {
      kind: "health",
      meeting_id: YT_MEETING_ID,
      channel: 1,
      state: "silent",
    });
    const quiet = page.getByTestId("yt-capture-quiet");
    await expect(quiet).toContainText("anderen Audioausgang");
    await expect(quiet).toContainText("auf Kopfhörer gewechselt");
  });

  // lva#538, Blocker 2: der Start haengt (der Treiber antwortet nicht); Abbrechen,
  // Escape und X sind derselbe Abbruch, kein bloßes Schliessen des Dialogs.
  test.describe("Abbruch während der Start läuft", () => {
    /** Dialog offen, Start abgeschickt, haengt. */
    const hangingStart = async (page: Page, extra: YoutubeMockOptions = {}) => {
      await setup(page, { empty: true, startHold: true, ...extra });
      await pickMeeting(page, YT_MEETING_ID);
      await startCapture(page);
      await expect.poll(async () => (await calls(page, START)).length).toBe(1);
      const dialog = captureDialog(page);
      // Der Dialog bleibt offen, solange der Start laeuft; Bestaetigen ist gesperrt.
      await expect(dialog).toBeVisible();
      await expect(dialog.getByTestId("yt-capture-confirm")).toBeDisabled();
      await expect(dialog.getByTestId("yt-capture-cancel")).toHaveText(
        "Start abbrechen",
      );
      return dialog;
    };

    /**
     * Der Abbruch ist wirksam: genau ein `meetings_cancel_start` mit der Vorgangs-Id des
     * Starts (kein bloßer Stopp, der den Start verfehlen koennte), Dialog zu, keine
     * Fehlermeldung, der Eintrag wie vorher.
     */
    const expectCancelled = async (page: Page) => {
      await expect
        .poll(async () => (await calls(page, "meetings_cancel_start")).length)
        .toBe(1);
      const start = (await calls(page, START))[0];
      const cancel = (await calls(page, "meetings_cancel_start"))[0];
      expect(typeof start.args.opId).toBe("string");
      expect((start.args.opId as string).length).toBeGreaterThan(0);
      expect(cancel.args.opId).toBe(start.args.opId);
      expect(await calls(page, "meetings_stop")).toHaveLength(0);
      await expect(captureDialog(page)).toHaveCount(0);
      await expect(page.getByTestId("yt-message")).toHaveCount(0);
      await expect(page.getByTestId("yt-capture-running")).toHaveCount(0);
      await expect(page.getByTestId("yt-capture")).toBeEnabled();
      // Der Player blieb unberuehrt, der Eintrag ist ohne Aufnahme wie vorher.
      expect(await playerCount(page)).toBe(0);
      await expect(
        page.getByText("Noch kein Transkript. Starte oben", { exact: false }),
      ).toBeVisible();
    };

    test("„Start abbrechen“ bricht genau diesen Start ab (über seine Vorgangs-Id)", async ({
      page,
    }) => {
      const dialog = await hangingStart(page);
      await dialog.getByTestId("yt-capture-cancel").click();
      await expectCancelled(page);
    });

    test("Escape ist derselbe Abbruch", async ({ page }) => {
      await hangingStart(page);
      await page.keyboard.press("Escape");
      await expectCancelled(page);
    });

    test("das X im Dialog ist derselbe Abbruch", async ({ page }) => {
      const dialog = await hangingStart(page);
      await dialog
        .getByRole("button", { name: "Abbrechen", exact: true })
        .click();
      await expectCancelled(page);
    });

    test("ein Klick auf den Hintergrund ist derselbe Abbruch", async ({
      page,
    }) => {
      await hangingStart(page);
      // Ausserhalb des Dialogs (Ecke der Ueberlagerung).
      await page.mouse.click(4, 4);
      await expectCancelled(page);
    });

    test("mehrfaches Abbrechen schickt nur einen Abbruch", async ({ page }) => {
      const dialog = await hangingStart(page);
      await dialog.getByTestId("yt-capture-cancel").click();
      await page.keyboard.press("Escape");
      await expectCancelled(page);
      expect(await calls(page, "meetings_cancel_start")).toHaveLength(1);
    });

    // Der Start haengt noch im Treiber, der Abbruch kommt mit `start_busy` zurueck: die
    // Oberflaeche sagt es, bleibt offen und laesst einen neuen Versuch zu (kein stiller
    // Erfolg, kein haengendes „Wird abgebrochen“).
    test("ein Abbruch, der nicht durchkommt, wird gemeldet und kann wiederholt werden", async ({
      page,
    }) => {
      await setup(page, {
        empty: true,
        startHold: true,
        cancelErrorOnce: "start_busy",
      });
      await pickMeeting(page, YT_MEETING_ID);
      await startCapture(page);
      await expect.poll(async () => (await calls(page, START)).length).toBe(1);
      const dialog = captureDialog(page);
      await dialog.getByTestId("yt-capture-cancel").click();
      await expect(page.getByTestId("yt-message")).toHaveText(
        "Der Start der Aufnahme läuft noch. Bitte gleich noch einmal versuchen.",
      );
      await expect(dialog).toBeVisible();
      await dialog.getByTestId("yt-capture-cancel").click();
      await expect(captureDialog(page)).toHaveCount(0);
      expect(await calls(page, "meetings_cancel_start")).toHaveLength(2);
    });

    // lva#538, Blocker F: die Live-Pipeline hat VOR dem Abbruch schon einen Satz gemeldet;
    // der Rollback verwirft ihn im Backend. Hier kommt KEIN Ereignis: die Oberflaeche muss
    // den gespeicherten Stand selbst neu laden (sonst bliebe der Phantomsatz stehen, der
    // Knopf „Beim Abspielen mitschreiben“ verschwaende und der Eintrag haette ein
    // „Transkript“, das es nicht gibt). Kein Warten auf Zeit: der Satz wird erst
    // gemeldet, wenn der Start haengt, und der Abbruch ist der einzige Ausloeser.
    const PHANTOM = "Phantomsatz vor dem Abbruch.";

    test("ein vor dem Abbruch gemeldeter Satz verschwindet, die Oberfläche lädt neu", async ({
      page,
    }) => {
      const dialog = await hangingStart(page);
      const phantom = page.getByText(PHANTOM);
      await captureSegment(page, 0, PHANTOM);
      await expect(phantom).toBeVisible();
      const loadsBefore = (await calls(page, "meetings_get_segments")).length;
      await dialog.getByTestId("yt-capture-cancel").click();
      await expectCancelled(page);
      await expect(phantom).toHaveCount(0);
      // Das Neuladen kam von der Oberflaeche selbst (es gab kein Ereignis).
      expect(
        (await calls(page, "meetings_get_segments")).length,
      ).toBeGreaterThan(loadsBefore);
    });

    // Dasselbe, wenn das Backend nach dem Rollback `reset` und `state ready` meldet
    // (`emit_restored`): der Eintrag ist leer, der Knopf wieder da.
    test("meldet das Backend den Rollback (reset und ready), ist der Eintrag leer", async ({
      page,
    }) => {
      const dialog = await hangingStart(page, { rollbackEmits: true });
      const phantom = page.getByText(PHANTOM);
      await captureSegment(page, 0, PHANTOM);
      await expect(phantom).toBeVisible();
      await dialog.getByTestId("yt-capture-cancel").click();
      await expectCancelled(page);
      await expect(phantom).toHaveCount(0);
    });

    // Der Abbruch kam erst nach der Veroeffentlichung an: der Start endet mit Erfolg, das
    // Backend beendet genau diese Aufnahme und setzt zurueck. Die Oberflaeche zeigt keine
    // laufende Aufnahme und laedt den gespeicherten Stand (ohne den Phantomsatz) neu.
    test("ein Abbruch nach der Veröffentlichung lädt den zurückgesetzten Stand", async ({
      page,
    }) => {
      const dialog = await hangingStart(page, { holdFinishesOnCancel: true });
      const phantom = page.getByText(PHANTOM);
      await captureSegment(page, 0, PHANTOM);
      await expect(phantom).toBeVisible();
      const loadsBefore = (await calls(page, "meetings_get_segments")).length;
      await dialog.getByTestId("yt-capture-cancel").click();
      await expectCancelled(page);
      await expect(phantom).toHaveCount(0);
      expect(
        (await calls(page, "meetings_get_segments")).length,
      ).toBeGreaterThan(loadsBefore);
    });
  });

  // lva#538, Hinweis Zeitmarken: bei eigenem Mitschnitt liegen die Zeitmarken auf der
  // Zeitachse der Aufnahme, nicht der Videozeit: kein Sprung im YouTube-Player.
  test("Zeitmarken eines Mitschnitts springen im eigenen Audio, nicht im YouTube-Player", async ({
    page,
  }) => {
    await setup(page, { ownAudio: true });
    await pickMeeting(page, YT_MEETING_ID);
    const first = page.locator('[data-segment-index="0"] [data-act="seek"]');
    await expect(first).toBeVisible();
    await first.click();
    // Kein YouTube-Player wurde geladen, kein Sprung zur Videozeit.
    await expect(page.getByTestId("yt-iframe")).toHaveCount(0);
    expect(await playerCount(page)).toBe(0);
    expect(await playerCalls(page)).toEqual([]);
    // Der Mitschnitt selbst ist abspielbar.
    await expect(page.getByTestId("rec-player-system")).toBeVisible();
  });

  test("läuft schon eine andere Aufnahme, ist der Knopf gesperrt", async ({
    page,
  }) => {
    await setup(page, { empty: true, recording: true });
    await pickMeeting(page, YT_MEETING_ID);
    await expect(page.getByTestId("yt-capture")).toBeDisabled();
    await expect(page.getByTestId("yt-capture")).toHaveAttribute(
      "title",
      "Es läuft schon eine andere Aufnahme.",
    );
    expect(await calls(page, START)).toHaveLength(0);
  });
});
