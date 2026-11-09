import { test, expect, type Page } from "@playwright/test";
import { geheZu } from "./profilmenue";

// Lokale Daten bei der Anmeldung (lva#400, Spec 2026-11-08): Modal mit Wahl,
// Abmelden-Dialog, Nur-lokal-Zeile und Sicherungen in „Konto & Geräte“.
// Der Kern wird im Browser nachgebildet; Sicherung, Prüfung und Journal
// prüfen die Rust-Tests (sync::uebernahme_kern). Nur synthetische Daten.
// Abnahmebilder: docs/vorschau/anmeldung-lokale-daten/.
const SHOTS = "docs/vorschau/anmeldung-lokale-daten";

const ABGEMELDET = {
  connected: false,
  user_email: null,
  device_name: null,
  hub_url: null,
  running: false,
  last_success_ms: null,
  last_error: null,
  pending: 0,
  dead_letters: 0,
  key_mismatch: false,
  pages: 0,
  nur_lokal: 0,
  anmeldung_offen: false,
  widerruf_offen: false,
  uebergang_fehler: null,
};
const ANGEMELDET = {
  ...ABGEMELDET,
  connected: true,
  user_email: "b@example.org",
  device_name: "Test-PC",
  hub_url: "https://portal.example.org",
  last_success_ms: 1794150000000,
  pages: 14,
  nur_lokal: 5,
};
const ENTSCHEIDUNG = {
  konto: "b@example.org",
  vorher_konto: null as string | null,
  seiten: 7,
  buecher: 2,
  fortschritte: 11,
  dateien: 2,
  einstellungen: true,
  sicherung_bytes: 48_500_000,
  kollisionen: [
    { titel: "Protokoll Jour fixe", art: "seite" },
    { titel: "Der Leuchtturm", art: "buch" },
  ],
};
const SICHERUNGEN = [
  {
    name: "konto-1794150000000",
    erstellt_ms: 1794150000000,
    anlass: "anmeldung",
    konto: "b@example.org",
    seiten: 7,
    buecher: 2,
    dateien: 2,
    bytes: 48_500_000,
    ordner: "/tmp/sicherungen/konto-1794150000000",
  },
  {
    name: "konto-1794050000000",
    erstellt_ms: 1794050000000,
    anlass: "abmeldung",
    konto: "a@example.org",
    seiten: 3,
    buecher: 0,
    dateien: 0,
    bytes: 1_200_000,
    ordner: "/tmp/sicherungen/konto-1794050000000",
  },
];

test.beforeEach(async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(() => {
    const callbacks = new Map<number, unknown>();
    let callback = 0;
    const settings = {
      onboarding_completed: true,
      app_language: "de",
      get theme() {
        return (window as any).__theme ?? "light";
      },
      show_whats_new_on_update: false,
      debug_mode: false,
      selected_model: "",
      bindings: {
        transcribe: {
          id: "transcribe",
          name: "Diktat",
          description: "",
          current_binding: "Ctrl+Space",
          default_binding: "Ctrl+Space",
        },
      },
      post_process_providers: [],
      post_process_prompts: [],
      custom_words: [],
      post_process_models: {},
      post_process_api_keys: {},
      push_to_talk: true,
    };
    Object.assign(window, {
      __TAURI_OS_PLUGIN_INTERNALS__: {
        platform: "macos",
        os_type: "macos",
        family: "unix",
        arch: "x86_64",
        version: "15.7.9",
        eol: "\n",
      },
      __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
      __TAURI_INTERNALS__: {
        metadata: {
          currentWindow: { label: "main" },
          currentWebview: { label: "main" },
        },
        transformCallback: (fn: unknown) => {
          callbacks.set(++callback, fn);
          return callback;
        },
        unregisterCallback: (id: number) => callbacks.delete(id),
        convertFileSrc: (path: string) => path,
        invoke: async (cmd: string, args?: Record<string, any>) => {
          // Suite-Profil (ADR-0024): zustandsbehafteter Kontospeicher.
          const w = window as any;
          w.__calls ??= [];
          w.__listeners ??= {};
          w.__konto ??= {
            aktiv: "p1",
            profile: [
              {
                id: "p1",
                anzeigename: "Lokales Profil",
                farbe: 15,
                hatBild: false,
                gesperrt: false,
                geaendertMs: 1,
                anmeldung: null,
              },
            ],
          };
          const dto = (p: any) => ({
            vorname: null,
            nachname: null,
            email: null,
            telefon: null,
            mobil: null,
            position: null,
            abteilung: null,
            organisation: null,
            standort: null,
            ...p,
          });
          const status = () => ({
            aktiv: dto(
              w.__konto.profile.find((p: any) => p.id === w.__konto.aktiv),
            ),
            weitere: w.__konto.profile
              .filter((p: any) => p.id !== w.__konto.aktiv)
              .map(dto),
            sitzungsrevision: 1,
          });
          if (cmd.startsWith("konto_")) w.__calls.push([cmd, args]);
          if (cmd === "plugin:event|listen") {
            (w.__listeners[args?.event] ??= []).push(args?.handler);
          }
          w.__senden = (event: string, payload: unknown) => {
            for (const id of w.__listeners[event] ?? []) {
              const fn = callbacks.get(id) as
                ((e: unknown) => void) | undefined;
              fn?.({ event, id, payload });
            }
          };
          w.__emit = (event: string) => {
            for (const id of w.__listeners[event] ?? []) {
              const fn = callbacks.get(id) as
                ((e: unknown) => void) | undefined;
              fn?.({ event, id, payload: null });
            }
          };
          if (cmd === "konto_status") return status();
          const nurAktiv = () => {
            if (args?.id !== w.__konto.aktiv)
              throw "Das aktive Profil hat sich geändert. Die Änderung wurde nicht übernommen.";
          };
          if (cmd === "konto_bild_lesen") {
            const p = w.__konto.profile.find((x: any) => x.id === args?.id);
            return p?.hatBild
              ? "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
              : null;
          }
          if (cmd === "konto_bild_entfernen") {
            nurAktiv();
            if (w.__bildEntfernenFehler) throw "kaputt";
            const p = w.__konto.profile.find((x: any) => x.id === args?.id);
            p.hatBild = false;
            p.geaendertMs += 1;
            return null;
          }
          if (cmd === "konto_aendern") {
            nurAktiv();
            const p = w.__konto.profile.find((x: any) => x.id === args?.id);
            for (const [k, v] of Object.entries(args?.aenderung ?? {})) {
              if (v !== null) p[k] = v === "" ? null : v;
            }
            return dto(p);
          }
          if (cmd === "konto_anlegen") {
            const p = {
              id: `p${w.__konto.profile.length + 1}`,
              anzeigename: args?.anzeigename,
              farbe: 3,
              hatBild: false,
              gesperrt: false,
              geaendertMs: 1,
              anmeldung: null,
            };
            w.__konto.profile.push(p);
            w.__konto.aktiv = p.id;
            return dto(p);
          }
          if (cmd === "konto_aktivieren") {
            w.__konto.aktiv = args?.id;
            return status();
          }
          if (cmd === "get_app_settings" || cmd === "get_default_settings")
            return settings;
          if (cmd === "plugin:os|locale") return "de-DE";
          if (cmd === "plugin:app|version") return "0.14.0";
          if (cmd.includes("permission")) return true;
          if (cmd === "plugin:event|listen") return ++callback;
          if (cmd === "get_selected_model") return "";
          if (cmd === "meetings_is_recording") return false;
          if (cmd === "tts_server_status")
            return { phase: "stopped", message: null };
          if (
            cmd === "pages_list" ||
            cmd === "page_files" ||
            cmd === "tts_list_voices" ||
            cmd === "tts_list_voice_infos" ||
            cmd === "llm_ps" ||
            cmd === "tts_reading_list" ||
            cmd === "tts_list_downloads" ||
            cmd === "llm_local_list"
          )
            return [];
          if (
            cmd.includes("history") ||
            cmd.includes("models") ||
            cmd.includes("devices") ||
            cmd === "meetings_list"
          )
            return [];
          if (cmd === "get_custom_sounds") return { start: false, stop: false };
          const s = (window as any).__sync;
          if (cmd.startsWith("sync_")) w.__calls.push([cmd, args]);
          if (cmd === "sync_status") return s.status;
          if (cmd === "sync_default_device_name") return "Test-PC";
          if (cmd === "sync_hub_status")
            return { hub_seq: 3, objects: { page: 2 }, devices: [] };
          if (cmd === "sync_login") {
            s.status = { ...s.status, anmeldung_offen: true };
            return { status: s.status, entscheidung: s.entscheidung };
          }
          if (cmd === "sync_anmeldung_offen")
            return s.status.anmeldung_offen ? s.entscheidung : null;
          if (
            cmd === "sync_anmeldung_entscheiden" ||
            cmd === "sync_nur_lokal_uebernehmen"
          ) {
            s.status = {
              ...s.status,
              anmeldung_offen: false,
              connected: true,
              user_email: "b@example.org",
              device_name: "Test-PC",
              hub_url: "https://portal.example.org",
              last_success_ms: Date.now(),
              nur_lokal:
                cmd === "sync_nur_lokal_uebernehmen" ? 0 : s.status.nur_lokal,
            };
            return s.status;
          }
          if (cmd === "sync_anmeldung_abbrechen") {
            s.status = { ...s.status, anmeldung_offen: false };
            return s.status;
          }
          if (cmd === "sync_logout") {
            s.status = { ...s.abgemeldet };
            return s.status;
          }
          if (cmd === "sync_sicherungen") return s.sicherungen;
          return null;
        },
      },
    });
    localStorage.setItem("lva.ui.settings.tab", "input");
  });
  await page.addInitScript(
    ([abgemeldet, angemeldet, entscheidung, sicherungen]) => {
      const w = window as any;
      // Aufbau erst nach den Testschaltern (__start, __wechsel).
      w.__baueSync = () => {
        w.__sync = {
          abgemeldet,
          status:
            w.__start === "angemeldet" ? { ...angemeldet } : { ...abgemeldet },
          entscheidung: w.__wechsel
            ? { ...entscheidung, vorher_konto: "a@example.org" }
            : entscheidung,
          sicherungen,
        };
        if (w.__startOffen) w.__sync.status.anmeldung_offen = true;
      };
      w.__baueSync();
    },
    [ABGEMELDET, ANGEMELDET, ENTSCHEIDUNG, SICHERUNGEN] as const,
  );
});

async function calls(page: Page): Promise<[string, Record<string, unknown>][]> {
  return page.evaluate(() => (window as any).__calls ?? []);
}

const vorbereiten = async (
  page: Page,
  opts: {
    start?: string;
    wechsel?: boolean;
    theme?: string;
    width?: number;
  } = {},
) => {
  const theme = opts.theme ?? "light";
  await page.emulateMedia({ colorScheme: theme === "dark" ? "dark" : "light" });
  await page.setViewportSize({ width: opts.width ?? 1440, height: 900 });
  await page.addInitScript(
    ([start, wechsel, th]) => {
      const w = window as any;
      w.__start = start;
      w.__wechsel = wechsel;
      w.__theme = th;
      w.__baueSync();
    },
    [opts.start ?? "abgemeldet", !!opts.wechsel, theme] as const,
  );
  await page.goto("/");
  await geheZu(page, "Einstellungen");
  await page.getByRole("tab", { name: "Konto & Geräte" }).click();
};

const anmelden = async (page: Page) => {
  const karte = page.getByTestId("sync-login");
  await karte.getByLabel("E-Mail").fill("b@example.org");
  await karte.getByLabel("Passwort", { exact: true }).fill("geheim-123");
  await karte.getByRole("button", { name: "Anmelden" }).click();
  await expect(page.getByTestId("anmeldung-daten")).toBeVisible();
};

const modal = (page: Page) => page.getByRole("alertdialog");

test("Anmeldung mit lokalem Bestand: Modal, Esc schließt nicht, Abbrechen ändert nichts", async ({
  page,
}) => {
  await vorbereiten(page);
  await anmelden(page);
  const m = modal(page);
  await expect(m).toContainText("Lokale Daten gefunden");
  await expect(m).toContainText("7 Seiten");
  await expect(m).toContainText("2 Bücher");
  await expect(m).toContainText("11 Lesestände und Lesezeichen");
  await expect(m).toContainText("Geänderte Einstellungen");
  await expect(m).toContainText(
    "Besprechungen, Diktate, Stimmen und Modelle bleiben auf diesem Gerät",
  );
  // Ohne Kontowechsel ist „zusammenführen“ vorausgewählt.
  await expect(
    m.getByRole("radio", { name: /Übernehmen und zusammenführen/ }),
  ).toBeChecked();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("anmeldung-daten")).toBeVisible();
  await m.getByTestId("anmeldung-abbrechen").click();
  await expect(page.getByTestId("anmeldung-daten")).toHaveCount(0);
  const c = (await calls(page)).map(([cmd]) => cmd);
  expect(c).toContain("sync_anmeldung_abbrechen");
  expect(c).not.toContain("sync_anmeldung_entscheiden");
  await expect(page.getByTestId("sync-login")).toBeVisible();
});

test("Sichern und getrennt halten: Kollisionen genannt, Wahl geht an den Kern", async ({
  page,
}) => {
  await vorbereiten(page);
  await anmelden(page);
  const m = modal(page);
  await expect(page.getByTestId("anmeldung-kollisionen")).toHaveCount(0);
  await m.getByRole("radio", { name: /Sichern und/ }).check();
  await expect(m.getByRole("radio", { name: /getrennt halten/ })).toBeChecked();
  await expect(page.getByTestId("anmeldung-kollisionen")).toContainText(
    "„Protokoll Jour fixe“ – bleibt als Kopie",
  );
  await expect(page.getByTestId("anmeldung-kollisionen")).toContainText(
    "„Der Leuchtturm“ – die lokale Fassung bleibt in der Sicherung",
  );
  await m.getByRole("button", { name: "Sichern und getrennt halten" }).click();
  await expect(page.getByTestId("anmeldung-daten")).toHaveCount(0);
  const e = (await calls(page)).find(
    ([cmd]) => cmd === "sync_anmeldung_entscheiden",
  );
  expect(e?.[1]).toEqual({
    wahl: "getrennt_halten",
    kopierenBestaetigt: false,
  });
});

test("Sichern und entfernen geht als Wahl „entfernen“ an den Kern", async ({
  page,
}) => {
  await vorbereiten(page);
  await anmelden(page);
  const m = modal(page);
  await m.getByRole("radio", { name: /Sichern und/ }).check();
  await m.getByRole("radio", { name: /vom Gerät entfernen/ }).check();
  await m.getByRole("button", { name: "Sichern und entfernen" }).click();
  const e = (await calls(page)).find(
    ([cmd]) => cmd === "sync_anmeldung_entscheiden",
  );
  expect(e?.[1]).toEqual({ wahl: "entfernen", kopierenBestaetigt: false });
});

test("Kontowechsel: getrennt halten vorausgewählt, Zusammenführen braucht das Häkchen", async ({
  page,
}) => {
  await vorbereiten(page, { wechsel: true });
  await anmelden(page);
  const m = modal(page);
  await expect(m).toContainText("Daten aus einem anderen Konto gefunden");
  await expect(m).toContainText("a@example.org");
  await expect(m.getByRole("radio", { name: /getrennt halten/ })).toBeChecked();
  await m.getByRole("radio", { name: /Übernehmen und zusammenführen/ }).check();
  const knopf = m.getByTestId("anmeldung-bestaetigen");
  await expect(knopf).toBeDisabled();
  await m
    .getByRole("checkbox", {
      name: "Daten aus a@example.org in b@example.org kopieren",
    })
    .check();
  await expect(knopf).toBeEnabled();
  await knopf.click();
  const e = (await calls(page)).find(
    ([cmd]) => cmd === "sync_anmeldung_entscheiden",
  );
  expect(e?.[1]).toEqual({ wahl: "zusammenfuehren", kopierenBestaetigt: true });
});

test("Wartende Anmeldung nach Neustart: Modal erscheint wieder", async ({
  page,
}) => {
  await page.addInitScript(() => {
    (window as any).__startOffen = true;
  });
  await vorbereiten(page);
  await expect(page.getByTestId("anmeldung-daten")).toBeVisible();
});

test("Abmelden: behalten vorausgewählt, entfernen sichert vorher", async ({
  page,
}) => {
  await vorbereiten(page, { start: "angemeldet" });
  await page.getByRole("button", { name: "Abmelden" }).click();
  const d = page.getByTestId("abmelden-dialog");
  await expect(d).toBeVisible();
  await expect(
    page.getByRole("radio", { name: /Daten auf diesem Gerät behalten/ }),
  ).toBeChecked();
  await page.getByRole("radio", { name: /vom Gerät entfernen/ }).check();
  await page.getByTestId("abmelden-bestaetigen").click();
  await expect(d).toHaveCount(0);
  const e = (await calls(page)).find(([cmd]) => cmd === "sync_logout");
  expect(e?.[1]).toEqual({ entfernen: true });
});

test("Nur lokal und Sicherungen: Zeile mit Übernahme, Liste mit Anlass", async ({
  page,
}) => {
  await vorbereiten(page, { start: "angemeldet" });
  await expect(page.getByTestId("sync-nur-lokal")).toContainText(
    "5 Einträge nur auf diesem Gerät",
  );
  const liste = page.getByTestId("sync-sicherungen");
  await expect(liste.getByRole("listitem")).toHaveCount(2);
  await expect(liste).toContainText("vor der Anmeldung");
  await expect(liste).toContainText("vor dem Abmelden");
  await page.getByRole("button", { name: "Ins Konto übernehmen" }).click();
  await expect(page.getByTestId("sync-nur-lokal")).toHaveCount(0);
  expect((await calls(page)).map(([c]) => c)).toContain(
    "sync_nur_lokal_uebernehmen",
  );
});

// Laufzeitnachweise (Codex 08.11.): Scrollbarkeit und Aktivierung bei 390 px,
// Fokusfalle, Fokusrückgabe und Touchflächen. Ergebnis: laufzeit.txt.
// Je Test eine eigene Datei: parallele Worker überschreiben sich so nicht.
const belegen = async (name: string, zeilen: string[]) => {
  const fs = await import("node:fs");
  fs.mkdirSync(SHOTS, { recursive: true });
  fs.writeFileSync(`${SHOTS}/laufzeit-${name}.txt`, zeilen.join("\n") + "\n");
};

test("390 px: Kollisionen scrollbar, Bestätigen erreichbar und aktiv", async ({
  page,
}) => {
  await vorbereiten(page, { width: 390, wechsel: true });
  await anmelden(page);
  const m = modal(page);
  await m.getByTestId("wahl-sichern").check();
  await m.getByTestId("wahl-getrennt").check();
  const koll = m.getByTestId("anmeldung-kollisionen");
  await expect(koll).toBeVisible();
  const letzte = koll.getByRole("listitem").last();
  await letzte.scrollIntoViewIfNeeded();
  await expect(letzte).toBeInViewport({ ratio: 1 });
  const knopf = m.getByTestId("anmeldung-bestaetigen");
  await knopf.scrollIntoViewIfNeeded();
  await expect(knopf).toBeInViewport({ ratio: 1 });
  await expect(knopf).toBeEnabled();
  const scroll = await page.evaluate(() => {
    const d = document.querySelector('[role="alertdialog"]') as HTMLElement;
    const el = [d, ...Array.from(d.querySelectorAll<HTMLElement>("*"))].find(
      (e) =>
        e.scrollHeight > e.clientHeight + 1 &&
        /(auto|scroll)/.test(getComputedStyle(e).overflowY),
    );
    return el
      ? { scrollHeight: el.scrollHeight, clientHeight: el.clientHeight }
      : null;
  });
  await page.screenshot({ path: `${SHOTS}/03b-kollisionen-gescrollt-390.png` });
  const anzahl = await koll.getByRole("listitem").count();
  expect(anzahl).toBeGreaterThan(0);
  const vorher = (await calls(page)).length;
  await knopf.click();
  await expect
    .poll(async () => (await calls(page)).length)
    .toBeGreaterThan(vorher);
  const neu = (await calls(page)).slice(vorher).map(([c]) => c);
  await belegen("390-scrollen", [
    `390 px Kontowechsel, „getrennt halten“: ${anzahl} Kollisionen, letzte vollständig im Bild nach Scrollen.`,
    `  Scrollbereich: ${scroll ? `${scroll.scrollHeight}px Inhalt in ${scroll.clientHeight}px` : "kein Überlauf (passt ganz)"}.`,
    `  Bestätigen: vollständig im Bild, aktiviert, Klick ruft ${neu.join(", ")} auf.`,
  ]);
});

test("Fokusfalle im Modal und Fokusrückgabe nach Abmelden-Dialog", async ({
  page,
}) => {
  await vorbereiten(page);
  await anmelden(page);
  const m = modal(page);
  const ausserhalb: string[] = [];
  for (let i = 0; i < 25; i++) {
    await page.keyboard.press(i % 5 === 4 ? "Shift+Tab" : "Tab");
    const drin = await page.evaluate(
      () => !!document.activeElement?.closest('[role="alertdialog"]'),
    );
    if (!drin) ausserhalb.push(String(i));
  }
  expect(ausserhalb).toEqual([]);
  await page.keyboard.press("Escape");
  await expect(m).toBeVisible();
  const fokus = [
    "Fokusfalle Anmelde-Modal: 25 Tab/Umschalt+Tab-Schritte, Fokus blieb in allen im Dialog; Esc schließt nicht.",
  ];
  await m.getByTestId("anmeldung-abbrechen").click();
  await page.evaluate(() => {
    const w = window as any;
    w.__sync.status = {
      ...w.__sync.status,
      connected: true,
      user_email: "b@example.org",
      device_name: "Test-PC",
      hub_url: "https://portal.example.org",
      anmeldung_offen: false,
    };
    w.__senden("sync-status", w.__sync.status);
  });
  const ab = page.getByRole("button", { name: "Abmelden" });
  await ab.focus();
  await ab.press("Enter");
  await expect(page.getByTestId("abmelden-dialog")).toBeVisible();
  const imDialog = await page.evaluate(
    () =>
      !!document.activeElement?.closest('[role="dialog"],[role="alertdialog"]'),
  );
  expect(imDialog).toBe(true);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("abmelden-dialog")).toHaveCount(0);
  await expect(ab).toBeFocused();
  await belegen("fokus", [
    ...fokus,
    "Abmelden-Dialog: Fokus beim Öffnen im Dialog; nach Esc zurück auf „Abmelden“.",
  ]);
});

test("Touchflächen mindestens 44 px (grober Zeiger)", async ({ page }) => {
  // Navigation breit, dann auf Telefonbreite: die Schublade ist nicht Prüfgegenstand.
  await vorbereiten(page, { wechsel: true });
  await page.setViewportSize({ width: 390, height: 844 });
  // Touch-Gerät nachbilden: in Chromium gilt danach (pointer: coarse).
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setTouchEmulationEnabled", {
    enabled: true,
    maxTouchPoints: 5,
  });
  await anmelden(page);
  const m = modal(page);
  await m.getByTestId("wahl-sichern").check();
  const grob = await page.evaluate(
    () => matchMedia("(pointer: coarse)").matches,
  );
  expect(grob).toBe(true);
  const ids = [
    "wahl-zusammenfuehren",
    "wahl-kopieren",
    "wahl-sichern",
    "wahl-getrennt",
    "wahl-entfernen",
    "anmeldung-bestaetigen",
    "anmeldung-abbrechen",
  ];
  const zeilen: string[] = [];
  for (const id of ids) {
    const ziel = m.getByTestId(id);
    if ((await ziel.count()) === 0) continue;
    // Bei Radio/Checkbox ist die Trefferfläche die ganze Zeile (Wurzel).
    const el = id.startsWith("wahl-") ? ziel.locator("..") : ziel;
    const b = await el.boundingBox();
    zeilen.push(`  ${id}: ${Math.round(b!.width)}×${Math.round(b!.height)} px`);
    expect(b!.height, id).toBeGreaterThanOrEqual(44);
  }
  await belegen("touch", [
    "Touchflächen bei 390 px mit Touch (pointer: coarse):",
    ...zeilen,
  ]);
});

for (const theme of ["light", "dark"]) {
  for (const width of [1440, 390]) {
    test(`Abnahmebilder ${theme} ${width}`, async ({ page }) => {
      await vorbereiten(page, { theme, width, wechsel: width === 390 });
      await anmelden(page);
      const m = modal(page);
      if (width === 1440) {
        await page.screenshot({
          path: `${SHOTS}/01-modal-${theme}-${width}.png`,
        });
        await m.getByRole("radio", { name: /Sichern und/ }).check();
        await page.screenshot({
          path: `${SHOTS}/02-getrennt-${theme}-${width}.png`,
        });
      } else {
        await page.screenshot({
          path: `${SHOTS}/03-kontowechsel-${theme}-${width}.png`,
        });
      }
      await m.getByTestId("anmeldung-abbrechen").click();
      await page.evaluate(() => {
        const w = window as any;
        w.__sync.status = {
          ...w.__sync.status,
          connected: true,
          user_email: "b@example.org",
          device_name: "Test-PC",
          hub_url: "https://portal.example.org",
          last_success_ms: Date.now() - 120000,
          pages: 14,
          nur_lokal: 5,
        };
      });
      // Stand wie nach einer Wahl: Ereignis „sync-status“ an die Karte.
      await page.evaluate(() => {
        const w = window as any;
        w.__sync.status.anmeldung_offen = false;
        w.__senden("sync-status", w.__sync.status);
      });
      await expect(page.getByTestId("sync-sicherungen")).toBeVisible();
      await page.getByTestId("sync-sicherungen").scrollIntoViewIfNeeded();
      await page.screenshot({
        path: `${SHOTS}/04-konto-${theme}-${width}.png`,
      });
      await page.getByRole("button", { name: "Abmelden" }).click();
      await expect(page.getByTestId("abmelden-dialog")).toBeVisible();
      await page.screenshot({
        path: `${SHOTS}/05-abmelden-${theme}-${width}.png`,
      });
    });
  }
}
