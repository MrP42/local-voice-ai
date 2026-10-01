import { test, expect, type Page } from "@playwright/test";
import { emlFileName, parseAddresses } from "../src/lib/meetingFollowup";

// M6-P6c (AK10 Teil 3): Follow-up-Mail gegen die Tauri-Attrappe. Jeder Aufruf
// landet in `window.__calls`; `meeting_followup_draft` bleibt offen, bis der
// Test ihn mit `__pending.resolve/reject` beendet; `meeting_followup_open`
// liefert, was `__openResult` vorgibt (Ergebnis oder Fehler).

type Call = { cmd: string; args: Record<string, any> };

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const w = window as any;
    const callbacks = new Map<number, (e: unknown) => void>();
    let callback = 0;
    const settings: Record<string, any> = {
      onboarding_completed: true,
      app_language: "de",
      theme: "light",
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: {},
      post_process_provider_id: "local",
      post_process_providers: [
        { id: "local", label: "Lokal", base_url: "http://127.0.0.1:0/v1" },
        {
          id: "openai",
          label: "OpenAI",
          base_url: "https://api.openai.com/v1",
        },
      ],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
      meeting_self_emails: [],
    };
    w.__settings = settings;
    w.__calls = [];
    w.__pending = null;
    w.__openResult = { ok: false };
    w.__savePath = "C:\\Temp\\Follow-up.eml";
    // A5: Microsoft-365-Konten fuer „Senden ueber“ und das Ergebnis des Versands.
    w.__integrations = [];
    w.__m365Result = { ok: true, code: "ok", detail: null };
    w.__meetings = [
      {
        id: "m1",
        title: "Kundentermin Meyer",
        status: "ready",
        source: "live",
        started_at: 1790000000,
        ended_at: 1790000600,
        language: "de",
        mic_audio_path: null,
        system_audio_path: null,
        duration_ms: 600000,
        consent_confirmed_at: 1790000000,
        audio_retention_until: null,
        created_at: 1790000000,
        source_path: null,
        deleted_at: null,
      },
    ];
    w.__segments = Array.from({ length: 5 }, (_, i) => ({
      segment_index: i,
      text: `Satz Nummer ${i}.`,
      start_ms: i * 15_000,
      end_ms: i * 15_000 + 5_000,
      channel: 0,
      speaker_index: null,
    }));

    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "windows",
        os_type: "windows",
        family: "windows",
        arch: "x86_64",
        version: "11",
        eol: "\r\n",
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: (fn: (e: unknown) => void) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (p: string) => p,
        invoke: async (cmd: string, args: Record<string, any> = {}) => {
          w.__calls.push({ cmd, args: JSON.parse(JSON.stringify(args)) });
          switch (cmd) {
            case "get_app_settings":
            case "get_default_settings":
              return settings;
            case "plugin:os|locale":
              return "de-DE";
            case "plugin:app|version":
              return "0.14.0";
            case "plugin:event|listen":
              return ++callback;
            case "plugin:event|unlisten":
              return null;
            case "meetings_is_recording":
              return false;
            case "meetings_recording_position":
              return null;
            case "meetings_list":
              return w.__meetings;
            case "meetings_search":
              return {
                items: w.__meetings.map((m: any) => ({
                  meeting: m,
                  snippet: null,
                  hit_source: null,
                })),
                total: w.__meetings.length,
                truncated: false,
              };
            case "meeting_folders_list":
              return [];
            case "meetings_get_segments":
              return w.__segments;
            case "meetings_segment_epoch":
              return 1;
            case "meetings_get_documents":
              return [];
            case "meeting_notes_get":
              return {
                meeting_id: args.meetingId,
                blocks: [],
                revision: 1,
                updated_at: 0,
              };
            case "meeting_followup_draft":
              return new Promise((resolve, reject) => {
                w.__pending = { meetingId: args.meetingId, resolve, reject };
              });
            case "meeting_followup_open": {
              const r = w.__openResult;
              if (r.ok) return r.value ?? false;
              throw r.error;
            }
            case "plugin:dialog|save":
              return w.__savePath;
            case "integrations_list":
              return w.__integrations;
            case "meeting_followup_send_m365":
              return w.__m365Result;
            case "get_selected_model":
              return "";
            case "tts_server_status":
              return { phase: "stopped", message: null };
            case "get_custom_sounds":
              return { start: false, stop: false };
            case "pages_list":
            case "page_files":
            case "tts_list_voices":
            case "tts_list_voice_infos":
            case "llm_ps":
            case "tts_reading_list":
            case "tts_list_downloads":
            case "llm_local_list":
            case "meeting_templates_list":
            case "action_items_list":
            case "chat_recipes_list":
            case "meeting_chat_threads":
              return [];
          }
          if (
            cmd.includes("permission") ||
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices")
          )
            return cmd.includes("permission") ? true : [];
          return null;
        },
      },
    });
    localStorage.setItem("lva.ui.settings.tab", "dictation");
  });
});

// ---------------------------------------------------------------------------
// Helfer
// ---------------------------------------------------------------------------

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) => (window as any).__calls.filter((x: Call) => x.cmd === c) as Call[],
    cmd,
  );

const draft = (over: Record<string, unknown> = {}) => ({
  to: ["anna@firma.de", "ben@firma.de"],
  subject: "Nächste Schritte zum Angebot",
  body_text: "Hallo Anna,\n\nvielen Dank für das Gespräch.",
  body_html: "<p>Hallo Anna,</p>",
  ...over,
});

const waitPending = (page: Page) =>
  expect
    .poll(() => page.evaluate(() => (window as any).__pending !== null))
    .toBe(true);

const resolveDraft = async (page: Page, value: unknown) => {
  await waitPending(page);
  await page.evaluate((v) => {
    const w = window as any;
    const pending = w.__pending;
    w.__pending = null;
    pending.resolve(v);
  }, value);
};

const rejectDraft = async (page: Page, error: string) => {
  await waitPending(page);
  await page.evaluate((e) => {
    const w = window as any;
    const pending = w.__pending;
    w.__pending = null;
    pending.reject(e);
  }, error);
};

const setOpenResult = (page: Page, result: Record<string, unknown>) =>
  page.evaluate((r) => ((window as any).__openResult = r), result);

const openDetail = async (page: Page, remote = false) => {
  if (remote) {
    await page.addInitScript(() => {
      (window as any).__settings.post_process_provider_id = "openai";
    });
  }
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto("/");
  await page.getByRole("button", { name: "Aufnahmen", exact: true }).click();
  await page.getByText("Kundentermin Meyer", { exact: true }).click();
  await expect(page.getByTestId("followup-open")).toBeEnabled();
};

const dialog = (page: Page) => page.getByTestId("followup-dialog");

/** Dialog öffnen (lokaler Anbieter) und den Entwurf liefern lassen. */
const openReady = async (page: Page, value: unknown = draft()) => {
  await openDetail(page);
  await page.getByTestId("followup-open").click();
  await resolveDraft(page, value);
  await expect(page.getByLabel("Betreff", { exact: true })).toBeVisible();
};

// ---------------------------------------------------------------------------
// Reine Logik
// ---------------------------------------------------------------------------

test.describe("meetingFollowup.ts (rein)", () => {
  test("parseAddresses trennt an Komma, Semikolon und Leerraum", () => {
    expect(parseAddresses("a@x.de, b@y.de;c@z.de  d@w.de")).toEqual([
      "a@x.de",
      "b@y.de",
      "c@z.de",
      "d@w.de",
    ]);
    expect(parseAddresses("  ")).toEqual([]);
  });

  test("emlFileName entfernt verbotene Zeichen und hat einen Rückfall", () => {
    expect(emlFileName('Angebot: "Q3" / Preise?')).toBe(
      "Angebot- -Q3- - Preise-.eml",
    );
    expect(emlFileName("   ")).toBe("Follow-up.eml");
    expect(emlFileName("x".repeat(200))).toBe(`${"x".repeat(80)}.eml`);
  });
});

// ---------------------------------------------------------------------------
// Dialog
// ---------------------------------------------------------------------------

test.describe("Follow-up-Mail", () => {
  test("lokaler Anbieter: Entwurf entsteht sofort, Felder sind gefüllt und bearbeitbar", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("followup-open").click();
    await expect(page.getByTestId("followup-loading")).toHaveText(
      "Entwurf wird geschrieben …",
    );
    await waitPending(page);
    expect(await page.evaluate(() => (window as any).__pending.meetingId)).toBe(
      "m1",
    );
    await expect(page.getByTestId("followup-remote-bar")).toHaveCount(0);
    await resolveDraft(page, draft());
    await expect(page.getByLabel("An", { exact: true })).toHaveValue(
      "anna@firma.de, ben@firma.de",
    );
    await expect(page.getByLabel("Betreff", { exact: true })).toHaveValue(
      "Nächste Schritte zum Angebot",
    );
    await expect(page.getByLabel("Text", { exact: true })).toHaveValue(
      "Hallo Anna,\n\nvielen Dank für das Gespräch.",
    );
    await expect(page.getByTestId("followup-loading")).toHaveCount(0);
    for (const id of ["copy", "mailto", "eml"]) {
      await expect(page.getByTestId(`followup-${id}`)).toBeEnabled();
    }
  });

  test("ohne Empfänger steht ein Hinweis unter „An“", async ({ page }) => {
    await openReady(page, draft({ to: [] }));
    await expect(page.getByLabel("An", { exact: true })).toHaveValue("");
    await expect(dialog(page)).toContainText(
      "Keine Teilnehmenden mit E-Mail-Adresse bekannt",
    );
    await page.getByLabel("An", { exact: true }).fill("neu@firma.de");
    await expect(dialog(page)).not.toContainText(
      "Keine Teilnehmenden mit E-Mail-Adresse bekannt",
    );
  });

  test("Kopieren sendet den bearbeiteten Entwurf und meldet Erfolg", async ({
    page,
  }) => {
    await openReady(page);
    await setOpenResult(page, { ok: true, value: false });
    await page.getByLabel("An", { exact: true }).fill("c@x.de; d@y.de");
    await page.getByLabel("Betreff", { exact: true }).fill("Geändert");
    await page.getByLabel("Text", { exact: true }).fill("Neuer Text");
    await page.getByTestId("followup-copy").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "In die Zwischenablage kopiert.",
    );
    const open = await calls(page, "meeting_followup_open");
    expect(open).toHaveLength(1);
    expect(open[0].args.mode).toBe("copy");
    expect(open[0].args.path).toBeNull();
    expect(open[0].args.draft).toMatchObject({
      to: ["c@x.de", "d@y.de"],
      subject: "Geändert",
      body_text: "Neuer Text",
    });
  });

  test("Mailprogramm: kurze Adresse öffnet still, lange legt den Text in die Zwischenablage", async ({
    page,
  }) => {
    await openReady(page);
    await setOpenResult(page, { ok: true, value: false });
    await page.getByTestId("followup-mailto").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "Im Mailprogramm geöffnet.",
    );
    expect((await calls(page, "meeting_followup_open"))[0].args.mode).toBe(
      "mailto",
    );

    await setOpenResult(page, { ok: true, value: true });
    await page.getByTestId("followup-mailto").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "liegt in der Zwischenablage – im Mailprogramm mit Strg+V einfügen",
    );
  });

  test("Mailprogramm nicht erreichbar: Fehlertext und Kopierknopf hervorgehoben", async ({
    page,
  }) => {
    await openReady(page);
    await setOpenResult(page, { ok: false, error: "mailto_failed" });
    const copy = page.getByTestId("followup-copy");
    await expect(copy).not.toHaveAttribute("data-highlighted", "true");
    await page.getByTestId("followup-mailto").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "Das Mailprogramm ließ sich nicht öffnen",
    );
    await expect(copy).toHaveAttribute("data-highlighted", "true");
    // Erfolgreiches Kopieren nimmt die Hervorhebung zurück.
    await setOpenResult(page, { ok: true, value: false });
    await copy.click();
    await expect(copy).not.toHaveAttribute("data-highlighted", "true");
  });

  test(".eml speichern: Speicherdialog, Pfad geht an den Befehl, Abbruch tut nichts", async ({
    page,
  }) => {
    await openReady(page);
    await setOpenResult(page, { ok: true, value: false });
    await page.evaluate(() => ((window as any).__savePath = null));
    await page.getByTestId("followup-eml").click();
    await expect
      .poll(async () => (await calls(page, "plugin:dialog|save")).length)
      .toBe(1);
    const dlg = (await calls(page, "plugin:dialog|save"))[0].args.options;
    expect(dlg.defaultPath).toBe("Nächste Schritte zum Angebot.eml");
    expect(dlg.filters[0].extensions).toEqual(["eml"]);
    expect(await calls(page, "meeting_followup_open")).toHaveLength(0);

    await page.evaluate(
      () => ((window as any).__savePath = "C:\\Temp\\Angebot.eml"),
    );
    await page.getByTestId("followup-eml").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "Gespeichert: C:\\Temp\\Angebot.eml",
    );
    const open = await calls(page, "meeting_followup_open");
    expect(open).toHaveLength(1);
    expect(open[0].args.mode).toBe("eml");
    expect(open[0].args.path).toBe("C:\\Temp\\Angebot.eml");
  });

  test("externer Anbieter: gelbe Leiste, Entwurf erst nach Klick", async ({
    page,
  }) => {
    await openDetail(page, true);
    await page.getByTestId("followup-open").click();
    await expect(page.getByTestId("followup-remote-bar")).toContainText(
      "OpenAI",
    );
    await expect(page.getByTestId("followup-remote-bar")).toContainText(
      "Auszüge der Besprechung werden an diesen Anbieter übermittelt",
    );
    // Ohne Klick läuft nichts.
    await page.waitForTimeout(300);
    expect(await calls(page, "meeting_followup_draft")).toHaveLength(0);
    await expect(page.getByTestId("followup-copy")).toHaveCount(0);
    await page.getByTestId("followup-generate").click();
    await resolveDraft(page, draft());
    await expect(page.getByLabel("Betreff", { exact: true })).toBeVisible();
    expect(await calls(page, "meeting_followup_draft")).toHaveLength(1);
    await expect(page.getByTestId("followup-remote-bar")).toBeVisible();
  });

  test("Fehler des Sprachmodells: Klartext, erneuter Versuch klappt", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("followup-open").click();
    await rejectDraft(page, "llm_failed: timeout");
    await expect(page.getByTestId("followup-error")).toHaveText(
      "Das Sprachmodell hat nicht geantwortet. Bitte erneut versuchen.",
    );
    await page.getByRole("button", { name: "Erneut versuchen" }).click();
    await resolveDraft(page, draft());
    await expect(page.getByLabel("Betreff", { exact: true })).toHaveValue(
      "Nächste Schritte zum Angebot",
    );
    expect(await calls(page, "meeting_followup_draft")).toHaveLength(2);
  });

  test("leere Antwort und kein Anbieter haben eigene Texte", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("followup-open").click();
    await rejectDraft(page, "followup_empty");
    await expect(page.getByTestId("followup-error")).toContainText(
      "keinen Text geliefert",
    );
    await page.getByRole("button", { name: "Erneut versuchen" }).click();
    await rejectDraft(page, "no_provider");
    await expect(page.getByTestId("followup-error")).toContainText(
      "Kein KI-Anbieter eingerichtet",
    );
  });

  test("keine Grundlage: klare Meldung, kein erneuter Versuch (B13)", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("followup-open").click();
    await rejectDraft(page, "followup_no_content");
    await expect(page.getByTestId("followup-error")).toHaveText(
      "Kein Inhalt für eine Nachbereitung gefunden – die Aufnahme enthält keine Besprechungspunkte.",
    );
    // Ein neuer Versuch änderte nichts: kein Knopf, kein weiterer Aufruf.
    await expect(
      page.getByRole("button", { name: "Erneut versuchen" }),
    ).toHaveCount(0);
    await page.waitForTimeout(400);
    expect(await calls(page, "meeting_followup_draft")).toHaveLength(1);
    // Schließen und wieder öffnen fragt neu (die Aufnahme könnte sich geändert haben).
    await page.getByRole("button", { name: "Schließen" }).last().click();
    await page.getByTestId("followup-open").click();
    await resolveDraft(page, draft());
    await expect(page.getByLabel("Betreff", { exact: true })).toBeVisible();
    expect(await calls(page, "meeting_followup_draft")).toHaveLength(2);
  });

  test("technische Fehler zeigen ihre eigene Meldung und bieten einen neuen Versuch", async ({
    page,
  }) => {
    await openDetail(page);
    await page.getByTestId("followup-open").click();
    await rejectDraft(page, "memory_low");
    await expect(page.getByTestId("followup-error")).toHaveText(
      "Zu wenig freier Arbeitsspeicher für das lokale Modell. Bitte andere Programme schließen.",
    );
    await expect(
      page.getByRole("button", { name: "Erneut versuchen" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Erneut versuchen" }).click();
    await rejectDraft(page, "no_model");
    await expect(page.getByTestId("followup-error")).toHaveText(
      "Für den KI-Anbieter ist kein Modell eingetragen.",
    );
    await page.getByRole("button", { name: "Erneut versuchen" }).click();
    await rejectDraft(page, "chat_busy");
    await expect(page.getByTestId("followup-error")).toContainText(
      "Es läuft schon eine Frage",
    );
  });

  test("Neu erzeugen ersetzt den Entwurf; Schließen verwirft eine späte Antwort", async ({
    page,
  }) => {
    await openReady(page);
    await page.getByTestId("followup-regenerate").click();
    await resolveDraft(page, draft({ subject: "Zweiter Entwurf" }));
    await expect(page.getByLabel("Betreff", { exact: true })).toHaveValue(
      "Zweiter Entwurf",
    );
    // Schließen, während ein Entwurf entsteht: die späte Antwort landet nirgends.
    await page.getByRole("button", { name: "Schließen" }).last().click();
    await expect(page.getByTestId("followup-dialog")).toHaveCount(0);
    await page.getByTestId("followup-open").click();
    await waitPending(page);
    await page.getByRole("button", { name: "Schließen" }).last().click();
    await resolveDraft(page, draft({ subject: "Zu spät" }));
    await page.getByTestId("followup-open").click();
    await resolveDraft(page, draft({ subject: "Frisch" }));
    await expect(page.getByLabel("Betreff", { exact: true })).toHaveValue(
      "Frisch",
    );
  });
});

// ---------------------------------------------------------------------------
// A5: Senden über Microsoft 365
// ---------------------------------------------------------------------------

const m365Account = (
  over: Record<string, unknown> = {},
  caps = ["mail.send"],
) => ({
  integration: {
    id: "m365-1",
    kind: "m365",
    label: "Mein Konto",
    enabled: true,
    direction: "both",
    config_json: JSON.stringify({ enabled_capabilities: caps }),
    ...over,
  },
});

const withAccounts = (page: Page, accounts: unknown[], result?: unknown) =>
  page.addInitScript(
    ({ accounts, result }) => {
      const w = window as any;
      w.__integrations = accounts;
      if (result) w.__m365Result = result;
    },
    { accounts, result },
  );

const m365Buttons = (page: Page) =>
  page.locator('[data-testid^="followup-m365-m365"]');

test.describe("Follow-up: senden über Microsoft 365", () => {
  test("ohne Microsoft-365-Konto, mit ausgeschaltetem Konto oder ohne „Mail senden“ gibt es keinen Knopf", async ({
    page,
  }) => {
    await withAccounts(page, [
      m365Account({ id: "m365-aus", enabled: false }),
      m365Account({ id: "m365-ohne" }, ["files.write"]),
      {
        integration: {
          id: "ordner",
          kind: "folder",
          label: "Ablage",
          enabled: true,
          config_json: "{}",
        },
      },
    ]);
    await openReady(page);
    await expect(m365Buttons(page)).toHaveCount(0);
    await expect(page.getByTestId("followup-mailto")).toBeVisible();
  });

  test("Senden fragt nach, schickt den bearbeiteten Entwurf und meldet den Erfolg", async ({
    page,
  }) => {
    await withAccounts(page, [m365Account()]);
    await openReady(page);
    await page
      .getByLabel("Betreff", { exact: true })
      .fill("Angepasster Betreff");
    const button = page.getByTestId("followup-m365-m365-1");
    await expect(button).toHaveText("Über Mein Konto senden");
    await button.click();
    // Erst die Rückfrage: es ist noch nichts gesendet.
    await expect(page.getByTestId("followup-m365-confirm")).toContainText(
      "an 2 Empfänger über „Mein Konto“",
    );
    expect(await calls(page, "meeting_followup_send_m365")).toHaveLength(0);
    await page.getByTestId("followup-m365-send").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "Gesendet über „Mein Konto“",
    );
    await expect(page.getByTestId("followup-m365-confirm")).toHaveCount(0);
    const sent = await calls(page, "meeting_followup_send_m365");
    expect(sent).toHaveLength(1);
    expect(sent[0].args).toEqual({
      integrationId: "m365-1",
      draft: {
        to: ["anna@firma.de", "ben@firma.de"],
        subject: "Angepasster Betreff",
        body_text: "Hallo Anna,\n\nvielen Dank für das Gespräch.",
        body_html: "",
      },
    });
  });

  test("Abbrechen in der Rückfrage sendet nichts", async ({ page }) => {
    await withAccounts(page, [m365Account()]);
    await openReady(page);
    await page.getByTestId("followup-m365-m365-1").click();
    await page.getByTestId("followup-m365-cancel").click();
    await expect(page.getByTestId("followup-m365-confirm")).toHaveCount(0);
    expect(await calls(page, "meeting_followup_send_m365")).toHaveLength(0);
  });

  test("ohne Empfänger ist der Knopf gesperrt", async ({ page }) => {
    await withAccounts(page, [m365Account()]);
    await openReady(page, draft({ to: [] }));
    await expect(page.getByTestId("followup-m365-m365-1")).toBeDisabled();
    await page.getByLabel("An", { exact: true }).fill("neu@firma.de");
    await expect(page.getByTestId("followup-m365-m365-1")).toBeEnabled();
  });

  test("abgelaufene Anmeldung und unklarer Ausgang stehen im Klartext da, der Entwurf bleibt", async ({
    page,
  }) => {
    await withAccounts(page, [m365Account()], {
      ok: false,
      code: "m365_needs_sign_in",
      detail: null,
    });
    await openReady(page);
    await page.getByTestId("followup-m365-m365-1").click();
    await page.getByTestId("followup-m365-send").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "abgelaufen oder wurde widerrufen",
    );
    await expect(page.getByLabel("Betreff", { exact: true })).toHaveValue(
      "Nächste Schritte zum Angebot",
    );
    await page.evaluate(() => {
      (window as any).__m365Result = {
        ok: false,
        code: "m365_uncertain|Zeitüberschreitung",
        detail: null,
      };
    });
    await page.getByTestId("followup-m365-m365-1").click();
    await page.getByTestId("followup-m365-send").click();
    await expect(page.getByTestId("followup-status")).toContainText(
      "Unklar, ob die Aktion ausgeführt wurde",
    );
  });
});
