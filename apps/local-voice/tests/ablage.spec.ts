import { test, expect, type Page, type TestInfo } from "@playwright/test";
import { execSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Seite „Ablage“ (lva#407 D1, lva#408 D2 Inhaltssuche): Abnahme-Belege. Die Befehle ablage_* sind mit
// synthetischen Daten belegt (nur erfundene Namen, keine echten Inhalte).
// Aufnahmen entstehen nur mit ABLAGE_SHOTS=1 (Verzeichnis docs/vorschau/ablage,
// überschreibbar mit LV_SHOT_DIR); ohne die Variable laufen nur die Prüfungen.
const SHOTS = process.env.ABLAGE_SHOTS === "1";
const SHOT_DIR = process.env.LV_SHOT_DIR ?? join("docs", "vorschau", "ablage");

type Theme = "light" | "dark";
const BREITEN = [390, 1440] as const;
const THEMEN: Theme[] = ["light", "dark"];
const HOEHE: Record<number, number> = { 390: 844, 1440: 900 };
const NAME_THEME: Record<Theme, string> = { light: "hell", dark: "dunkel" };

interface Befund {
  bild: string;
  scrollBreit: number;
  fenster: number;
  unter44: string[];
}
const befunde: Befund[] = [];

const T0 = 1_793_000_000_000;
const TAG = 86_400_000;

const ki = (
  zustand: string,
  von: string | null = null,
  von_name: string | null = null,
  vorgabe = false,
) => ({ zustand, von, von_name, vorgabe });

const ordner = (
  id: string,
  name: string,
  eltern: string | null,
  dateien: number,
  extra: Record<string, unknown> = {},
) => ({
  id,
  name,
  eltern,
  wiederhergestellt: false,
  datenklasse: "erbt",
  klasse: "internal",
  nur_geraet: false,
  nur_geraet_wirksam: false,
  zielordner_ki: false,
  ki: ki("aus", null, null, true),
  dateien,
  ...extra,
});

const datei = (
  id: string,
  name: string,
  ord: string,
  typ: string,
  groesse: number,
  tage: number,
  extra: Record<string, unknown> = {},
) => ({
  id,
  name,
  ordner: ord,
  typ,
  groesse,
  geaendert: T0 - tage * TAG,
  fassungen: 1,
  konflikt: false,
  klasse: "internal",
  ki: ki("aus", null, null, true),
  nur_geraet: false,
  text: "ok",
  text_code: null,
  ...extra,
});

const KI_AN = ki("an");
const KI_SPERRE = ki("gesperrt");
const KI_GEERBT = ki("gesperrt", "o_proj", "Projekte");

function demoSicht() {
  return {
    lage: null,
    freigabe_unklar: null,
    ordner: [
      ordner("o_allg", "Allgemein", null, 5),
      ordner("o_vert", "Verträge", null, 4, {
        ki: KI_AN,
        klasse: "confidential",
        datenklasse: "confidential",
      }),
      ordner("o_proj", "Projekte", null, 0, { ki: KI_SPERRE }),
      ordner("o_bruecke", "Brücke Nord", "o_proj", 2, {
        ki: KI_GEERBT,
        nur_geraet: true,
        nur_geraet_wirksam: true,
      }),
    ],
    dateien: [
      datei(
        "d1",
        "Protokoll Jour fixe 2026-11.docx",
        "o_allg",
        "docx",
        48_200,
        1,
      ),
      datei("d2", "Haushaltsplan 2027.xlsx", "o_allg", "xlsx", 312_000, 3, {
        konflikt: true,
        fassungen: 3,
      }),
      datei("d3", "Anfrage Bürgerbüro.eml", "o_allg", "eml", 9_400, 5),
      datei("d4", "Notizen Klausurtagung.md", "o_allg", "md", 3_100, 8),
      datei("d5", "Organigramm Amt 20.pdf", "o_allg", "pdf", 1_840_000, 21),
      datei("d6", "Rahmenvertrag Wartung.pdf", "o_vert", "pdf", 920_000, 14, {
        ki: KI_AN,
        klasse: "confidential",
      }),
      datei(
        "d7",
        "Dienstleistungsvertrag Reinigung.docx",
        "o_vert",
        "docx",
        61_000,
        30,
        {
          ki: KI_AN,
          klasse: "confidential",
        },
      ),
      datei(
        "d8",
        "Kündigung Mietvertrag Lager.pdf",
        "o_vert",
        "pdf",
        210_000,
        45,
        {
          ki: KI_AN,
          klasse: "confidential",
        },
      ),
      datei(
        "d9",
        "Datenschutzerklärung Entwurf.pdf",
        "o_vert",
        "pdf",
        133_000,
        60,
        {
          ki: KI_AN,
          klasse: "confidential",
        },
      ),
      datei(
        "d10",
        "Statikgutachten Brücke Nord.pdf",
        "o_bruecke",
        "pdf",
        4_200_000,
        9,
        {
          ki: KI_GEERBT,
          nur_geraet: true,
        },
      ),
      datei(
        "d11",
        "Bauzeitenplan Brücke Nord.xlsx",
        "o_bruecke",
        "xlsx",
        87_000,
        12,
        {
          ki: KI_GEERBT,
          nur_geraet: true,
        },
      ),
    ],
    papierkorb: [
      {
        art: "datei",
        id: "k1",
        name: "Entwurf Kooperationsvereinbarung.docx",
        typ: "docx",
        geloescht: T0 - 2 * TAG,
        tage_rest: 28,
        ursprung: "Verträge",
      },
      {
        art: "ordner",
        id: "k2",
        name: "Alte Notizen",
        typ: "",
        geloescht: T0 - 26 * TAG,
        tage_rest: 4,
        ursprung: "Allgemein",
      },
    ],
    gefunden: [],
    belegt: 7_825_700,
  };
}

function leereSicht() {
  return {
    lage: null,
    freigabe_unklar: null,
    ordner: [ordner("o_allg", "Allgemein", null, 0)],
    dateien: [],
    papierkorb: [],
    gefunden: [],
    belegt: 0,
  };
}

async function installMock(
  page: Page,
  opts: { theme: Theme; leer?: boolean; wahl?: string[]; sync?: unknown },
) {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ sicht, theme, wahl, sync }) => {
      const callbacks = new Map<number, unknown>();
      let callback = 0;
      const settings = {
        onboarding_completed: true,
        app_language: "de",
        theme,
        show_whats_new_on_update: false,
        debug_mode: false,
        selected_model: "",
        bindings: {},
        post_process_providers: [],
        post_process_prompts: [],
        custom_words: [],
        post_process_models: {},
        post_process_api_keys: {},
        push_to_talk: true,
        tts_engine: "piper",
      };
      const w = window as unknown as Record<string, unknown>;
      w.__calls = [] as unknown[];
      w.__sicht = sicht;
      w.__pick = wahl;
      w.__sync = sync;
      const listeners: Record<string, number[]> = {};
      window.localStorage.setItem("lva.ui.section", "ablage");
      Object.assign(window, {
        __TAURI_OS_PLUGIN_INTERNALS__: {
          platform: "windows",
          os_type: "windows",
          family: "windows",
          arch: "x86_64",
          version: "10.0.26200",
          eol: "\r\n",
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
          convertFileSrc: (path: string) => `asset://${path}`,
          invoke: async (cmd: string, args?: Record<string, any>) => {
            (w.__calls as unknown[]).push([cmd, args]);
            const s = w.__sicht as {
              ordner: any[];
              dateien: any[];
              papierkorb: any[];
              belegt: number;
            };
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") {
              (listeners[String(args?.event)] ??= []).push(
                Number(args?.handler),
              );
              return Number(args?.handler);
            }
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "plugin:dialog|open") return w.__pick ?? null;
            if (cmd === "ablage_stand") return JSON.parse(JSON.stringify(s));
            // D4: Sync-Zustand (ohne Angabe: kein Konto).
            if (cmd === "ablage_sync_stand")
              return (
                w.__sync ?? {
                  verbunden: false,
                  letzter_lauf_ms: null,
                  fehler: null,
                  kontingent_voll: false,
                  belegt: null,
                  kontingent: null,
                  eintraege: [],
                }
              );
            // D2: Inhaltssuche mit synthetischen Fundstellen (Anker nach Markenformat v1).
            if (cmd === "ablage_suchen") {
              const q = String(args?.anfrage ?? "").toLowerCase();
              if (!q.includes("kündigung")) return [];
              return [
                {
                  dat: "d7",
                  name: "Dienstleistungsvertrag Reinigung.docx",
                  ordner: "o_vert",
                  typ: "docx",
                  rang: 0.9,
                  stellen: [
                    {
                      anker: "absatz=4",
                      pfad: ["Dienstleistungsvertrag", "§ 4 Laufzeit"],
                      auszug:
                        "Die Kündigungsfrist beträgt drei Monate zum Quartalsende.",
                    },
                  ],
                },
                {
                  dat: "d2",
                  name: "Haushaltsplan 2027.xlsx",
                  ordner: "o_allg",
                  typ: "xlsx",
                  rang: 0.5,
                  stellen: [
                    {
                      anker: "blatt=Kosten%202027&zeilen=21-40",
                      pfad: ["Kosten 2027"],
                      auszug: "Posten: Kündigung Wartungsvertrag; Betrag: 1200",
                    },
                  ],
                },
              ];
            }
            if (cmd === "ablage_fassungen")
              return [
                {
                  fas: "f2",
                  nummer: 2,
                  vorgaenger: "f1",
                  groesse: 48_200,
                  typ: "docx",
                  zeit: 1_793_000_000_000 - 86_400_000,
                  aktuell: true,
                  kopf: true,
                  abgeloest: false,
                },
                {
                  fas: "f1",
                  nummer: 1,
                  vorgaenger: null,
                  groesse: 44_900,
                  typ: "docx",
                  zeit: 1_793_000_000_000 - 9 * 86_400_000,
                  aktuell: false,
                  kopf: false,
                  abgeloest: false,
                },
              ];
            if (cmd === "ablage_importieren") {
              const quelle = String(args?.pfad);
              const name = quelle.split(/[\\/]/).pop() ?? quelle;
              const vorhanden = s.dateien.some(
                (d) => d.ordner === args?.ordner && d.name === name,
              );
              if (vorhanden && !args?.wahl)
                return [
                  {
                    quelle,
                    stand: "kollision",
                    ordner: args?.ordner,
                    dat: null,
                    fas: null,
                    name,
                    code: null,
                  },
                ];
              if (!vorhanden) {
                s.dateien.push({
                  id: `neu_${s.dateien.length}`,
                  name,
                  ordner: args?.ordner,
                  typ: name.split(".").pop() ?? "",
                  groesse: 52_000,
                  geaendert: Date.now(),
                  fassungen: 1,
                  konflikt: false,
                  klasse: "internal",
                  ki: {
                    zustand: "aus",
                    von: null,
                    von_name: null,
                    vorgabe: true,
                  },
                  nur_geraet: false,
                  text: null,
                  text_code: null,
                });
              }
              return [
                {
                  quelle,
                  stand: vorhanden ? "neue_fassung" : "abgelegt",
                  ordner: args?.ordner,
                  dat: "x",
                  fas: "y",
                  name,
                  code: null,
                },
              ];
            }
            if (cmd === "ablage_ordner_anlegen") {
              const id = `o_neu${s.ordner.length}`;
              s.ordner.push({
                id,
                name: args?.name,
                eltern: args?.eltern,
                wiederhergestellt: false,
                datenklasse: "erbt",
                klasse: "internal",
                nur_geraet: false,
                nur_geraet_wirksam: false,
                zielordner_ki: false,
                ki: {
                  zustand: "aus",
                  von: null,
                  von_name: null,
                  vorgabe: true,
                },
                dateien: 0,
              });
              return id;
            }
            if (cmd === "ablage_wiederherstellen") {
              s.papierkorb = s.papierkorb.filter((k) => k.id !== args?.id);
              return null;
            }
            // D6: semantische Suche (Opt-in), Modell zunächst nicht geladen.
            if (cmd.startsWith("ablage_semantik_")) {
              const sem = ((w.__semantik as Record<string, unknown>) ??= {
                an: false,
                standard: false,
                modell_bereit: false,
                laedt: false,
                prozent: 0,
                reste: false,
                groesse_mb: 488,
                ladefehler: false,
                nachziehen: false,
                vektoren_fehlen: 0,
                neuaufbau: false,
                index_fehler: false,
                aktiv: false,
              });
              if (cmd === "ablage_semantik_setzen") sem.an = !!args?.an;
              if (cmd === "ablage_semantik_laden" && w.__semantikFehler)
                throw "semantik.download: Das Modell konnte nicht geladen werden.";
              if (cmd === "ablage_semantik_laden") sem.modell_bereit = true;
              if (cmd === "ablage_semantik_loeschen") sem.modell_bereit = false;
              sem.aktiv = !!sem.an && !!sem.modell_bereit;
              return { ...sem };
            }
            if (cmd.startsWith("ablage_")) return null;
            if (cmd === "tts_server_status")
              return { phase: "stopped", message: null };
            if (cmd === "get_custom_sounds")
              return { start: false, stop: false };
            if (cmd === "meetings_is_recording") return false;
            if (cmd === "get_selected_model") return "";
            if (cmd.endsWith("_list") || cmd === "llm_ps") return [];
            if (
              cmd.includes("models") ||
              cmd.includes("history") ||
              cmd.includes("devices")
            )
              return [];
            return null;
          },
        },
      });
    },
    {
      sicht: opts.leer ? leereSicht() : demoSicht(),
      theme: opts.theme,
      wahl: opts.wahl ?? null,
      sync: opts.sync ?? null,
    },
  );
}

async function offne(page: Page, breite: number, theme: Theme) {
  await page.setViewportSize({ width: breite, height: HOEHE[breite] });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.getByTestId("ablage-seite")).toBeVisible();
  // Einblendungen und Übergänge würden Aufnahmen und Maße verfälschen.
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;transition-delay:0s!important}",
  });
}

/** Wählt Ordner (`o_…`) oder `ablage-korb`; unter 1024 px über das Ordnerblatt. */
async function waehle(page: Page, id: string) {
  const ziel = id === "ablage-korb" ? "ablage-korb" : `ablage-ordner-${id}`;
  const knopf = page.getByTestId("ablage-ordner-waehlen");
  if (await knopf.isVisible()) {
    await knopf.click();
    await page.getByRole("dialog").getByTestId(ziel).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    return;
  }
  await page.getByTestId(ziel).click();
}

const SUCHFELD = (page: Page) =>
  page.getByPlaceholder("Namen und Inhalte durchsuchen");

/** Im schmalen Fenster liegt die Suche hinter einem Knopf der Titelleiste. */
async function suchfeld(page: Page) {
  const feld = SUCHFELD(page);
  if (await feld.isVisible()) return feld;
  await page.getByRole("button", { name: /suche/i }).first().click();
  await expect(feld).toBeVisible();
  return feld;
}

/** Sichtbare Bedienflächen (Maße in px) – optional nur innerhalb von `wurzel`. */
async function bedienflaechen(page: Page, wurzel: string | null) {
  return page.evaluate((root) => {
    const sel =
      "button, a[href], input:not([type=hidden]), select, textarea, [role=radio], [role=switch], [role=tab], [role=combobox], [role=checkbox]";
    const nodes = Array.from(
      (root ? document.querySelectorAll(root) : [document]).length
        ? (root
            ? Array.from(document.querySelectorAll(root))
            : [document]
          ).flatMap((r) => Array.from((r as ParentNode).querySelectorAll(sel)))
        : [],
    ) as HTMLElement[];
    const seen = new Set<HTMLElement>();
    const out: { name: string; w: number; h: number }[] = [];
    for (const el of nodes) {
      if (seen.has(el)) continue;
      seen.add(el);
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      if (r.width === 0 || r.height === 0) continue;
      if (el instanceof HTMLInputElement && r.width < 8) continue;
      if (cs.visibility === "hidden" || cs.display === "none") continue;
      if (el.closest("[aria-hidden=true]")) continue;
      if ((el as HTMLButtonElement).disabled) continue;
      const name =
        el.getAttribute("aria-label") ||
        el.getAttribute("data-testid") ||
        (el.textContent ?? "").trim().slice(0, 40) ||
        el.tagName;
      out.push({ name, w: Math.round(r.width), h: Math.round(r.height) });
    }
    return out;
  }, wurzel);
}

const zuKlein = (l: { name: string; w: number; h: number }[]) =>
  l.filter((x) => x.w < 44 || x.h < 44).map((x) => `${x.name} (${x.w}×${x.h})`);

async function breiteOk(page: Page) {
  const m = await page.evaluate(() => ({
    scroll: document.documentElement.scrollWidth,
    fenster: window.innerWidth,
  }));
  expect(m.scroll).toBeLessThanOrEqual(m.fenster);
  return m;
}

async function aufnahme(
  page: Page,
  screen: string,
  breite: number,
  theme: Theme,
  wurzel: string,
) {
  const bild = `${screen}-${breite}-${NAME_THEME[theme]}.png`;
  const m = await breiteOk(page);
  const flaechen = breite < 1024 ? await bedienflaechen(page, wurzel) : [];
  befunde.push({
    bild,
    scrollBreit: m.scroll,
    fenster: m.fenster,
    unter44: zuKlein(flaechen),
  });
  if (!SHOTS) return;
  mkdirSync(SHOT_DIR, { recursive: true });
  await page.screenshot({ path: join(SHOT_DIR, bild) });
}

test.afterAll(() => {
  if (!SHOTS || befunde.length === 0) return;
  mkdirSync(SHOT_DIR, { recursive: true });
  let kopf = "unbekannt";
  try {
    kopf = execSync("git rev-parse --short HEAD").toString().trim();
  } catch {
    /* ohne git */
  }
  const ziel = join(SHOT_DIR, "belege.json");
  let alt: Befund[] = [];
  try {
    alt = JSON.parse(readFileSync(ziel, "utf8")).befunde ?? [];
  } catch {
    /* erste Datei */
  }
  const neu = new Set(befunde.map((b) => b.bild));
  const sortiert = [...alt.filter((b) => !neu.has(b.bild)), ...befunde].sort(
    (a, b) => a.bild.localeCompare(b.bild),
  );
  writeFileSync(
    ziel,
    JSON.stringify({ kopf, befunde: sortiert }, null, 2) + "\n",
  );
});

// --- Aufnahmen und Grundprüfungen ---------------------------------------------------------
for (const theme of THEMEN) {
  for (const breite of BREITEN) {
    test.describe(`Ablage ${breite} px ${NAME_THEME[theme]}`, () => {
      const ROOT = '[data-testid="ablage-seite"]';

      test("liste: Kopf, Baum, Zeilen, kein Seitwärtsrollen", async ({
        page,
      }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await expect(
          page.getByRole("heading", { level: 1, name: "Ablage" }),
        ).toHaveCount(1);
        if (breite >= 1024) {
          await expect(
            page.getByRole("navigation", { name: /Ordner/i }),
          ).toBeVisible();
          await expect(page.getByTestId("ablage-ordner-o_allg")).toBeVisible();
          await expect(
            page.getByTestId("ablage-ordner-o_bruecke"),
          ).toBeVisible();
          await expect(page.getByTestId("ablage-korb")).toContainText("2");
        } else {
          await expect(page.getByTestId("ablage-ordner-waehlen")).toBeVisible();
        }
        await waehle(page, "o_allg");
        await expect(page.getByTestId("ablage-ordnername")).toHaveText(
          "Allgemein",
        );
        await expect(page.getByTestId(/^ablage-datei-/)).toHaveCount(5);
        await expect(page.getByTestId("ablage-datei-d2")).toBeVisible();
        await expect(
          page.getByRole("img", { name: "Konflikt" }).first(),
        ).toBeVisible();
        // keine rohen Übersetzungsschlüssel
        await expect(page.getByTestId("ablage-seite")).not.toContainText(
          /\bablage\.[a-z]+\./,
        );
        if (breite >= 1024) {
          await page.getByTestId("ablage-datei-d1").click();
          await expect(page.getByTestId("ablage-seite-details")).toBeVisible();
        }
        await aufnahme(page, "liste", breite, theme, ROOT);
      });

      test("hochladen: Liste mit Namenskollision", async ({ page }) => {
        await installMock(page, {
          theme,
          wahl: [
            "C:/Eingang/Angebot Wartung 2027.pdf",
            "C:/Eingang/Haushaltsplan 2027.xlsx",
          ],
        });
        await offne(page, breite, theme);
        await waehle(page, "o_allg");
        await page.getByTestId("ablage-hochladen").click();
        await expect(page.getByTestId("ablage-upload-kollision")).toBeVisible();
        await expect(page.getByTestId("ablage-upload-abgelegt")).toBeVisible();
        await expect(
          page.getByTestId("ablage-kollision-fassung"),
        ).toBeVisible();
        await aufnahme(
          page,
          "hochladen",
          breite,
          theme,
          breite < 1024 ? '[role="dialog"]' : ROOT,
        );
      });

      test("ordner: Einstellungsdialog", async ({ page }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, "o_vert");
        await page.getByTestId("ablage-ordner-einstellungen").click();
        const dlg = page.getByRole("dialog");
        await expect(dlg).toBeVisible();
        await expect(dlg).toContainText("KI");
        await expect(dlg).not.toContainText(/\bablage\.[a-z]+\./);
        await aufnahme(page, "ordner", breite, theme, '[role="dialog"]');
      });

      test("suche: Filter nach Namen", async ({ page }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, "o_vert");
        await expect(page.getByTestId(/^ablage-datei-/)).toHaveCount(4);
        const feld = await suchfeld(page);
        await feld.fill("vertrag");
        await expect(page.getByTestId(/^ablage-datei-/)).toHaveCount(3);
        await expect(page.getByTestId("ablage-datei-d9")).toHaveCount(0);
        await feld.fill("zzz");
        await expect(page.getByTestId("ablage-leer")).toContainText(
          "Keine Datei passt",
        );
        await feld.fill("vertrag");
        await expect(page.getByTestId(/^ablage-datei-/)).toHaveCount(3);
        await aufnahme(page, "suche", breite, theme, ROOT);
      });

      test("suche: Treffer im Inhalt mit Fundstelle", async ({ page }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, "o_allg");
        const feld = await suchfeld(page);
        await feld.fill("Kündigung");
        const ergebnisse = page.getByTestId("ablage-suchergebnisse");
        await expect(ergebnisse).toBeVisible();
        // Namenstreffer aus einem anderen Ordner, Inhaltstreffer mit lesbarem Anker.
        await expect(
          ergebnisse.getByTestId("ablage-namenstreffer-d8"),
        ).toBeVisible();
        const stelle = ergebnisse.getByTestId("ablage-inhaltstreffer-d7");
        await expect(stelle).toContainText(
          "Absatz 4 · Dienstleistungsvertrag › § 4 Laufzeit",
        );
        await expect(stelle.locator("mark").first()).toHaveText(/kündigung/i);
        await expect(
          ergebnisse.getByTestId("ablage-inhaltstreffer-d2"),
        ).toContainText("Blatt „Kosten 2027“, Zeilen 21–40");
        await expect(ergebnisse).not.toContainText(/\bablage\.[a-z]+\./);
        expect(
          await page.evaluate(() =>
            (
              (window as unknown as { __calls: [string, unknown][] }).__calls ??
              []
            ).some(([c]) => c === "ablage_suchen"),
          ),
        ).toBe(true);
        await aufnahme(page, "inhaltssuche", breite, theme, ROOT);
        // Öffnen: Ordner der Datei, Details mit Fundstelle.
        await stelle.click();
        const fund = page.getByTestId("ablage-fundstelle");
        await expect(fund).toBeVisible();
        await expect(fund).toContainText("Absatz 4");
        await expect(fund).toContainText("Kündigungsfrist");
        // Ohne Inhaltstreffer: Hinweis statt Liste.
        if (breite < 1024) await page.keyboard.press("Escape");
        await feld.fill("zzz");
        await expect(page.getByTestId("ablage-inhalt-leer")).toBeVisible();
      });

      test("suche: semantische Suche einschalten und Modell laden", async ({
        page,
      }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, "o_allg");
        const feld = await suchfeld(page);
        await feld.fill("Kündigung");
        await expect(
          page.getByTestId("ablage-inhaltstreffer-d7"),
        ).toBeVisible();
        const zeile = page.getByTestId("ablage-semantik");
        await expect(zeile).toBeVisible();
        const status = page.getByTestId("ablage-semantik-status");
        await expect(status).toContainText("wörtlich gesucht");
        await page.getByTestId("ablage-semantik-schalter").click();
        await expect(status).toContainText("Modell nicht geladen (488 MB)");
        await expect(status).toContainText("Noch nicht Standard");
        await page.getByTestId("ablage-semantik-laden").click();
        await expect(status).toContainText("Aktiv");
        await expect(page.getByTestId("ablage-semantik-laden")).toHaveCount(0);
        await expect(zeile).not.toContainText(/\bablage\.[a-z]+\./);
        const aufrufe = () =>
          page.evaluate(
            () =>
              (
                (window as unknown as { __calls: [string, unknown][] })
                  .__calls ?? []
              ).filter(([c]) => c === "ablage_suchen").length,
          );
        // Jeder Wechsel der Suchart wiederholt die Inhaltssuche (verzögert).
        const vorher = await aufrufe();
        await page.getByTestId("ablage-semantik-schalter").click();
        await expect(status).toContainText("wörtlich gesucht");
        await expect.poll(aufrufe).toBeGreaterThan(vorher);
        await page.getByTestId("ablage-semantik-schalter").click();
        await expect(status).toContainText("Aktiv");
        await aufnahme(page, "semantik", breite, theme, ROOT);
        // Modell entfernen: zurück auf wörtlich, Laden wird wieder angeboten.
        await page.getByTestId("ablage-semantik-entfernen").click();
        await expect(status).toContainText("Modell nicht geladen (488 MB)");
        await expect(page.getByTestId("ablage-semantik-laden")).toBeVisible();
        await expect(page.getByTestId("ablage-semantik-entfernen")).toHaveCount(
          0,
        );
      });

      test("papierkorb: zwei Einträge", async ({ page }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, "ablage-korb");
        await expect(page.getByTestId("ablage-ordnername")).toHaveText(
          "Papierkorb",
        );
        await expect(
          page.getByRole("button", { name: /wiederherstellen/i }),
        ).toHaveCount(2);
        await expect(page.getByTestId("ablage-seite")).toContainText(
          "Alte Notizen",
        );
        await aufnahme(page, "papierkorb", breite, theme, ROOT);
      });

      test("leer: leerer Ordner", async ({ page }) => {
        await installMock(page, { theme, leer: true });
        await offne(page, breite, theme);
        await expect(page.getByTestId("ablage-leer")).toContainText(
          "Noch keine Dateien",
        );
        await aufnahme(page, "leer", breite, theme, ROOT);
      });
    });
  }
}

// --- Maße und Bedienflächen ---------------------------------------------------------------
test("390 px: Bedienflächen der Ablage mindestens 44×44, Shell wird berichtet", async ({
  page,
}, info: TestInfo) => {
  await installMock(page, { theme: "light" });
  await offne(page, 390, "light");
  await waehle(page, "o_allg");
  const seite = zuKlein(
    await bedienflaechen(page, '[data-testid="ablage-seite"]'),
  );
  const alle = zuKlein(await bedienflaechen(page, null));
  const shell = alle.filter((x) => !seite.includes(x));
  info.annotations.push({
    type: "Shell unter 44 px (nicht Teil der Ablage)",
    description: shell.join("; ") || "keine",
  });
  console.log(`Shell unter 44 px: ${shell.join("; ") || "keine"}`);
  expect(seite, `Ablage unter 44 px: ${seite.join("; ")}`).toEqual([]);
});

for (const breite of BREITEN) {
  test(`${breite} px: kein Seitwärtsrollen in Liste, Papierkorb und Dialog`, async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, breite, "light");
    await waehle(page, "o_allg");
    await breiteOk(page);
    await waehle(page, "ablage-korb");
    await breiteOk(page);
    await waehle(page, "o_bruecke");
    await breiteOk(page);
  });
}

// --- Dialoge: Escape und Fokus ------------------------------------------------------------
for (const breite of BREITEN) {
  test.describe(`Escape und Fokus ${breite} px`, () => {
    test("Ordnereinstellungen: Escape schließt, Fokus zurück zum Auslöser", async ({
      page,
    }) => {
      await installMock(page, { theme: "light" });
      await offne(page, breite, "light");
      await waehle(page, "o_vert");
      const ausloeser = page.getByTestId("ablage-ordner-einstellungen");
      await ausloeser.focus();
      await page.keyboard.press("Enter");
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(ausloeser).toBeFocused();
    });

    test("Neuer Ordner: Escape schließt, Fokus zurück zum Auslöser", async ({
      page,
    }) => {
      await installMock(page, { theme: "light" });
      await offne(page, breite, "light");
      const ausloeser =
        breite < 1024
          ? page.getByRole("button", { name: "Neuer Ordner" })
          : page.getByTestId("ablage-ordner-neu");
      await ausloeser.focus();
      await page.keyboard.press("Enter");
      await expect(page.getByTestId("ablage-name-feld")).toBeFocused();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(ausloeser).toBeFocused();
    });

    test("Papierkorb leeren: Escape schließt, Fokus zurück", async ({
      page,
    }) => {
      await installMock(page, { theme: "light" });
      await offne(page, breite, "light");
      await waehle(page, "ablage-korb");
      const ausloeser = page.getByTestId("ablage-leeren");
      await ausloeser.focus();
      await page.keyboard.press("Enter");
      await expect(page.getByRole("dialog")).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(ausloeser).toBeFocused();
    });
  });
}

test("390 px: Dateiblatt schließt mit Escape, Fokus zurück zur Zeile", async ({
  page,
}) => {
  await installMock(page, { theme: "light" });
  await offne(page, 390, "light");
  await waehle(page, "o_allg");
  const zeile = page.getByTestId("ablage-datei-d1");
  await zeile.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(zeile).toBeFocused();
});

test("390 px: Hochladeblatt schließt mit Escape, Fokus zurück zu „Hochladen“", async ({
  page,
}) => {
  await installMock(page, {
    theme: "light",
    wahl: ["C:/Eingang/Angebot Wartung 2027.pdf"],
  });
  await offne(page, 390, "light");
  await waehle(page, "o_allg");
  const ausloeser = page.getByTestId("ablage-hochladen");
  await ausloeser.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(ausloeser).toBeFocused();
});

// --- Ordnerauswahl unter 1024 px ----------------------------------------------------------
for (const theme of THEMEN) {
  test(`390 px ${NAME_THEME[theme]}: Ordnerwahl per Tastatur, Blatt mit Fokusrückgabe`, async ({
    page,
  }) => {
    await installMock(page, { theme });
    await offne(page, 390, theme);
    await expect(page.getByTestId("ablage-ordner-o_allg")).toBeHidden();
    const knopf = page.getByTestId("ablage-ordner-waehlen");
    await knopf.focus();
    await page.keyboard.press("Enter");
    const dlg = page.getByRole("dialog", { name: "Ordner" });
    await expect(dlg).toBeVisible();
    await expect(dlg).toHaveAttribute("aria-modal", "true");
    await expect(dlg.getByTestId("ablage-ordner-o_bruecke")).toBeVisible();
    await expect(dlg.getByTestId("ablage-korb")).toBeVisible();
    // Fokusfalle: Tab verlässt den Dialog nicht.
    for (let i = 0; i < 12; i++) await page.keyboard.press("Tab");
    expect(
      await page.evaluate(
        () => !!document.activeElement?.closest('[role="dialog"]'),
      ),
    ).toBe(true);
    const flaechen = zuKlein(await bedienflaechen(page, '[role="dialog"]'));
    expect(flaechen, `Blatt unter 44 px: ${flaechen.join("; ")}`).toEqual([]);
    await aufnahme(page, "ordnerwahl", 390, theme, '[role="dialog"]');
    // Escape: schließt, Fokus zurück zum Auslöser.
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(knopf).toBeFocused();
    // Wahl per Tastatur: Ordner öffnet, Blatt schließt.
    await page.keyboard.press("Enter");
    await dlg.getByTestId("ablage-ordner-o_vert").focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByTestId("ablage-ordnername")).toHaveText("Verträge");
  });
}

test("1440 px: Schließen der Desktopdetails gibt den Fokus an die Zeile zurück", async ({
  page,
}) => {
  await installMock(page, { theme: "light" });
  await offne(page, 1440, "light");
  await waehle(page, "o_allg");
  const zeile = page.getByTestId("ablage-datei-d1");
  await zeile.focus();
  await page.keyboard.press("Enter");
  const details = page.getByTestId("ablage-seite-details");
  await expect(details).toBeVisible();
  await details.getByRole("button", { name: "Details schließen" }).click();
  await expect(details).toHaveCount(0);
  await expect(zeile).toBeFocused();
});

test("1440 px: semantische Suche – Fortschritt als Balken, Fehler als Hinweis, Status ohne Zahl", async ({
  page,
}) => {
  await installMock(page, { theme: "light" });
  // Download läuft schon (z. B. nach Neuöffnen der Seite), danach scheitert ein neuer Versuch.
  await page.addInitScript(() => {
    (window as unknown as Record<string, unknown>).__semantik = {
      an: true,
      standard: false,
      modell_bereit: false,
      laedt: true,
      prozent: 42,
      reste: false,
      groesse_mb: 488,
      ladefehler: false,
      nachziehen: false,
      vektoren_fehlen: 0,
      neuaufbau: false,
      index_fehler: false,
      aktiv: false,
    };
  });
  await offne(page, 1440, "light");
  await waehle(page, "o_allg");
  const feld = await suchfeld(page);
  await feld.fill("Kündigung");
  const status = page.getByTestId("ablage-semantik-status");
  await expect(status).toContainText("Modell wird geladen");
  await expect(status).not.toContainText("%");
  const balken = page.getByRole("progressbar");
  await expect(balken).toHaveAttribute("aria-valuenow", "42");
  await expect(page.getByTestId("ablage-semantik-abbrechen")).toBeVisible();
  // Download beendet mit Fehler: halbe Datei bleibt, Hinweis als Alarm, Entfernen wird angeboten.
  await page.evaluate(() => {
    const w = window as unknown as Record<string, Record<string, unknown>>;
    Object.assign(w.__semantik, { laedt: false, prozent: 0, reste: true });
    (w as unknown as Record<string, unknown>).__semantikFehler = true;
  });
  await page.getByTestId("ablage-semantik-schalter").click();
  await page.getByTestId("ablage-semantik-schalter").click();
  await expect(page.getByRole("progressbar")).toHaveCount(0);
  await expect(page.getByTestId("ablage-semantik-entfernen")).toBeVisible();
  await page.getByTestId("ablage-semantik-laden").click();
  await expect(
    page.getByRole("alert").filter({ hasText: "Verbindung" }),
  ).toContainText("Das Modell konnte nicht geladen werden");
  // Zustände ohne Vektoren: eigener Text statt „Noch 0 Textstellen“.
  await page.evaluate(() => {
    const w = window as unknown as Record<string, Record<string, unknown>>;
    Object.assign(w.__semantik, { modell_bereit: true, index_fehler: true });
  });
  await page.getByTestId("ablage-semantik-schalter").click();
  await page.getByTestId("ablage-semantik-schalter").click();
  await expect(status).toContainText("Suchindex gerade nicht verfügbar");
  await page.evaluate(() => {
    const w = window as unknown as Record<string, Record<string, unknown>>;
    Object.assign(w.__semantik, { index_fehler: false, neuaufbau: true });
  });
  await page.getByTestId("ablage-semantik-schalter").click();
  await page.getByTestId("ablage-semantik-schalter").click();
  await expect(status).toContainText(
    "muss für die semantische Suche erweitert",
  );
  await expect(status).not.toContainText("Noch 0");
});

test("1440 px: Sync-Zustand je Datei und Kontingent im Fuß (D4)", async ({
  page,
}) => {
  await installMock(page, {
    theme: "light",
    sync: {
      verbunden: true,
      letzter_lauf_ms: T0,
      fehler: "Ablage: Objekte mit anderem Schlüssel im Konto",
      kontingent_voll: true,
      belegt: 5_000_000,
      kontingent: 5_000_000,
      eintraege: [
        { id: "d2", zustand: "synchronisiert" },
        { id: "d3", zustand: "wartet_auf_text" },
        { id: "d4", zustand: "konflikt" },
        { id: "d5", zustand: "nur_geraet" },
      ],
    },
  });
  await offne(page, 1440, "light");
  await waehle(page, "o_allg");
  const zeile = (id: string) =>
    page.locator("tr", { has: page.getByTestId(`ablage-datei-${id}`) });
  // Ohne Sync-Eintrag: neutral „Noch nicht erfasst“.
  await expect(
    zeile("d1").getByRole("img", { name: "Noch nicht erfasst" }),
  ).toBeVisible();
  // Ein lokaler Konflikt hat Vorrang vor dem Sync-Eintrag.
  await expect(
    zeile("d2").getByRole("img", { name: "Konflikt" }),
  ).toBeVisible();
  await expect(
    zeile("d2").getByRole("img", { name: "Synchronisiert" }),
  ).toHaveCount(0);
  await expect(
    zeile("d3").getByRole("img", { name: "Wartet auf Text" }),
  ).toBeVisible();
  await expect(
    zeile("d4").getByRole("img", { name: "Konflikt" }),
  ).toBeVisible();
  await expect(
    zeile("d5").getByRole("img", { name: "Nur dieses Gerät" }),
  ).toBeVisible();
  await expect(page.getByTestId("ablage-fuss-cloud")).toHaveText(
    "Cloud-Speicher voll – Hochladen angehalten",
  );
  // Der letzte Fehler steht zusätzlich da.
  await expect(page.getByTestId("ablage-fuss-fehler")).toHaveText(
    "Abgleich unvollständig",
  );
  await page.screenshot({ path: join(SHOT_DIR, "d4-sync-1440-light.png") });
});
