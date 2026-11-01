import { test, expect, type Page } from "@playwright/test";

// Suite-Profil oben rechts (ADR-0024, Paket 1): Profilknopf mit Menue statt des
// alten Platzhalter-Menues, Ansicht "Mein Profil", Reiter "Konto & Geraete" in den
// Einstellungen (mit der Sync-Karte aus "Allgemein"). Der Kontospeicher wird im
// Browser nachgebildet; die Rust-Seite prueft das Windows-Tor.
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
      theme: "light",
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
          w.__emit = (event: string) => {
            for (const id of w.__listeners[event] ?? []) {
              const fn = callbacks.get(id) as
                ((e: unknown) => void) | undefined;
              fn?.({ event, id, payload: null });
            }
          };
          if (cmd === "konto_status") return status();
          if (cmd === "konto_aendern") {
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
              anmeldung: null,
            };
            w.__konto.profile.push(p);
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
          // Geraete-Sync: abgemeldet, bis der Test sich anmeldet.
          const syncState = {
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
          };
          if (cmd === "sync_status")
            return (window as any).__syncState ?? syncState;
          if (cmd === "sync_default_device_name") return "Test-PC";
          if (cmd === "sync_login") {
            (window as any).__syncState = {
              ...syncState,
              connected: true,
              user_email: "p@wai.test",
              device_name: "Test-PC",
              hub_url: "https://portal.wolffappliedai.de",
              last_success_ms: Date.now(),
              pages: 2,
            };
            return (window as any).__syncState;
          }
          if (cmd === "sync_hub_status")
            return {
              hub_seq: 3,
              objects: { page: 2 },
              devices: [
                { device: "abcdef123456", last_push: "2026-09-14 12:00:00" },
              ],
            };
          if (cmd === "sync_logout") {
            (window as any).__syncState = syncState;
            return syncState;
          }
          return null;
        },
      },
    });
    localStorage.setItem("lva.ui.settings.tab", "input");
  });
});

async function calls(page: Page): Promise<[string, Record<string, unknown>][]> {
  return page.evaluate(() => (window as any).__calls ?? []);
}

test("Profilknopf: lokales Profil, kein Anmelden-Knopf, Mein Profil oeffnet die Ansicht", async ({
  page,
}) => {
  await page.goto("/");
  const bar = page.locator(".lv-titlebar");
  await bar.getByRole("button", { name: "Profil: Lokales Profil" }).click();
  const menu = page.locator(".wai-konto-menue");
  await expect(menu).toContainText("Lokales Profil");
  // Paket 1 hat keine Anmeldung: weder Knopf noch Hinweis.
  await expect(menu.getByRole("button", { name: "Anmelden" })).toHaveCount(0);
  await menu.getByRole("button", { name: "Mein Profil" }).click();
  await expect(page.locator("#page-title")).toHaveText("Mein Profil");
  await expect(page.locator("h1")).toHaveCount(1);
  // Nicht in der Seitenleiste.
  const nav = page.getByRole("navigation", { name: "Hauptnavigation" });
  await expect(nav.getByRole("button", { name: "Mein Profil" })).toHaveCount(0);
});

test("Mein Profil: Anzeigename aendern speichert ueber konto_aendern", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .locator(".lv-titlebar")
    .getByRole("button", { name: /^Profil:/ })
    .click();
  await page
    .locator(".wai-konto-menue")
    .getByRole("button", { name: "Mein Profil" })
    .click();
  const feld = page.getByLabel("Anzeigename");
  await feld.fill("Anna Beispiel");
  await page.getByLabel("E-Mail").fill("anna@example.org");
  await page.getByRole("button", { name: "Speichern" }).click();
  await expect(page.getByText("Gespeichert.")).toBeVisible();
  const call = (await calls(page)).find(([c]) => c === "konto_aendern");
  expect(call?.[1]).toMatchObject({
    id: "p1",
    aenderung: {
      anzeigename: "Anna Beispiel",
      email: "anna@example.org",
      vorname: null,
    },
  });
  await expect(
    page
      .locator(".lv-titlebar")
      .getByRole("button", { name: "Profil: Anna Beispiel" }),
  ).toBeVisible();
});

test("Aenderung einer anderen App (Ereignis konto-geaendert) aktualisiert den Knopf", async ({
  page,
}) => {
  await page.goto("/");
  const bar = page.locator(".lv-titlebar");
  await expect(
    bar.getByRole("button", { name: "Profil: Lokales Profil" }),
  ).toBeVisible();
  await page.evaluate(() => {
    const w = window as any;
    w.__konto.profile[0].anzeigename = "Berta Muster";
    w.__emit("konto-geaendert");
  });
  await expect(
    bar.getByRole("button", { name: "Profil: Berta Muster" }),
  ).toBeVisible();
});

test("Konto & Geraete im Menue oeffnet den neuen Reiter", async ({ page }) => {
  await page.goto("/");
  await page
    .locator(".lv-titlebar")
    .getByRole("button", { name: /^Profil:/ })
    .click();
  await page
    .locator(".wai-konto-menue")
    .getByRole("button", { name: "Konto & Geräte" })
    .click();
  await expect(page.locator("#page-title")).toHaveText("Einstellungen");
  await expect(
    page.getByRole("tab", { name: "Konto & Geräte" }),
  ).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("konto-profilliste")).toBeVisible();
});

test("Reiter: Konto & Geraete zwischen Allgemein und Ueber, Sync-Karte zieht um", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  const tabs = page.getByRole("tablist").first().getByRole("tab");
  const namen = await tabs.allInnerTexts();
  const i = namen.indexOf("Allgemein");
  expect(namen.slice(i, i + 3)).toEqual([
    "Allgemein",
    "Konto & Geräte",
    "Über",
  ]);

  await page.getByRole("tab", { name: "Allgemein" }).click();
  await expect(page.getByTestId("sync-login")).toHaveCount(0);

  await page.getByRole("tab", { name: "Konto & Geräte" }).click();
  await expect(page.getByTestId("sync-login")).toBeVisible();
  const liste = page.getByTestId("konto-profilliste");
  await expect(liste.getByRole("listitem")).toHaveCount(1);
  await expect(liste).toContainText("Aktiv");
});

test("Profil hinzufuegen: Hinweis, anlegen, aktivieren, wechseln", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Einstellungen", exact: true })
    .click();
  await page.getByRole("tab", { name: "Konto & Geräte" }).click();
  await page.getByTestId("konto-profil-hinzufuegen").click();
  await expect(page.getByTestId("konto-trennung-hinweis")).toHaveText(
    "Profile am selben Windows-Konto sind nicht voreinander geschützt. Für getrennte Daten bitte eigene Windows-Konten nutzen.",
  );
  await page.getByLabel("Anzeigename").fill("Chris");
  await page.getByTestId("konto-anlegen").click();
  const liste = page.getByTestId("konto-profilliste");
  await expect(liste.getByRole("listitem")).toHaveCount(2);
  const aufrufe = (await calls(page)).map(([c]) => c);
  expect(aufrufe).toContain("konto_anlegen");
  expect(aufrufe).toContain("konto_aktivieren");
  await expect(liste.locator("li[data-aktiv]")).toContainText("Chris");

  // Zurueck zum ersten Profil ueber den Profilknopf.
  await page
    .locator(".lv-titlebar")
    .getByRole("button", { name: "Profil: Chris" })
    .click();
  await page
    .locator(".wai-konto-menue")
    .getByRole("button", { name: "Zu Lokales Profil wechseln" })
    .click();
  await expect(
    page
      .locator(".lv-titlebar")
      .getByRole("button", { name: "Profil: Lokales Profil" }),
  ).toBeVisible();
  expect(
    (await calls(page)).filter(([c]) => c === "konto_aktivieren").length,
  ).toBe(2);
});
