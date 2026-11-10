import { test, expect, type Page } from "@playwright/test";
import { execSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// Ablage D7-Rest (lva#395): Cloud-Ausnahme je Ordner, Unterordnerbaum verknüpfter Ordner, „Nur online“.
// Alle Namen, Ordner und Anbieter sind erfunden („Beispiel-Cloud EU“, „Muster-KI Europa“,
// „Demo-Anbieter USA“). Aufnahmen entstehen nur mit ABLAGE_D7_SHOTS=1 (Verzeichnis
// docs/vorschau/ablage-d7-rest/umsetzung, überschreibbar mit LV_SHOT_DIR); ohne die Variable laufen
// nur die Prüfungen.
const SHOTS = process.env.ABLAGE_D7_SHOTS === "1";
const SHOT_DIR =
  process.env.LV_SHOT_DIR ??
  join("docs", "vorschau", "ablage-d7-rest", "umsetzung");

type Theme = "light" | "dark";
const THEMEN: Theme[] = ["light", "dark"];
const NAME_THEME: Record<Theme, string> = { light: "hell", dark: "dunkel" };
const BREITEN = [1440, 768, 390] as const;
const HOEHE: Record<number, number> = { 1440: 900, 768: 1024, 390: 844 };

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
  verknuepft: false,
  cloud: { anbieter: [] as string[], geerbt: [] as string[] },
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
  verknuepft: false,
  cloud: { anbieter: [] as string[], geerbt: [] as string[] },
  ...extra,
});

const verknuepftOrdner = (id: string, name: string, dateien: number) =>
  ordner(id, name, null, dateien, {
    datenklasse: "confidential",
    klasse: "confidential",
    nur_geraet: true,
    nur_geraet_wirksam: true,
    verknuepft: true,
  });
const vdatei = (
  id: string,
  name: string,
  ord: string,
  typ: string,
  groesse: number,
  tage: number,
  text: string,
) =>
  datei(id, name, ord, typ, groesse, tage, {
    klasse: "confidential",
    nur_geraet: true,
    verknuepft: true,
    text,
  });

function demoSicht() {
  return {
    lage: null,
    freigabe_unklar: null,
    freigabe_wiederhergestellt: false,
    ordner: [
      ordner("o_allg", "Allgemein", null, 3),
      ordner("o_vert", "Verträge", null, 2, {
        ki: ki("an"),
        datenklasse: "confidential",
        klasse: "confidential",
      }),
      ordner("o_wart", "Wartung IT", "o_vert", 2, {
        klasse: "confidential",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      ordner("o_miete", "Miete", "o_vert", 1, {
        klasse: "confidential",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      ordner("o_gehalt", "Gehälter", "o_vert", 1, {
        datenklasse: "personal",
        klasse: "personal",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      ordner("o_proj", "Projekte", null, 0),
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
      datei("d2", "Haushaltsplan 2027.xlsx", "o_allg", "xlsx", 312_000, 3),
      datei("d3", "Organigramm Amt 20.pdf", "o_allg", "pdf", 1_840_000, 21),
      datei("d4", "Rahmenvertrag Wartung.pdf", "o_vert", "pdf", 920_000, 14, {
        klasse: "confidential",
        ki: ki("an"),
      }),
      datei(
        "d5",
        "Dienstleistungsvertrag Reinigung.docx",
        "o_vert",
        "docx",
        61_000,
        30,
        {
          klasse: "confidential",
          ki: ki("an"),
        },
      ),
      datei("d6", "Wartungsplan Serverraum.xlsx", "o_wart", "xlsx", 77_000, 9, {
        klasse: "confidential",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      datei("d7", "Störungsprotokoll 2026.pdf", "o_wart", "pdf", 150_000, 11, {
        klasse: "confidential",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      datei("d8", "Mietvertrag Lager.pdf", "o_miete", "pdf", 210_000, 45, {
        klasse: "confidential",
        ki: ki("an", "o_vert", "Verträge"),
      }),
      datei("d9", "Gehaltsliste Muster.xlsx", "o_gehalt", "xlsx", 30_000, 60, {
        klasse: "personal",
        ki: ki("an", "o_vert", "Verträge"),
      }),
    ],
    papierkorb: [],
    gefunden: [],
    belegt: 3_700_000,
    verknuepft: [
      {
        ordner: verknuepftOrdner("ord_od", "OneDrive – Dokumente", 6),
        erreichbar: true,
        ausgelassen: 0,
        nur_online: 3,
        zu_viele: false,
        abgetastet: T0,
        unterordner: [
          { pfad: "Projektakten 2026", nur_online: false },
          { pfad: "Projektakten 2026/Angebote", nur_online: false },
          { pfad: "Projektakten 2026/Protokolle", nur_online: false },
          { pfad: "Rechnungen 2025", nur_online: true },
          { pfad: "Vorlagen", nur_online: false },
        ],
      },
      {
        ordner: verknuepftOrdner("ord_arch", "Archiv (Netzlaufwerk)", 1),
        erreichbar: false,
        ausgelassen: 0,
        nur_online: 0,
        zu_viele: false,
        abgetastet: T0 - 3 * TAG,
        unterordner: [{ pfad: "2019", nur_online: false }],
      },
    ],
    verknuepft_dateien: [
      vdatei("vd1", "Notizen Begehung.md", "ord_od", "md", 3_100, 5, "ok"),
      vdatei(
        "vd2",
        "Plan Grundriss.jpg",
        "ord_od",
        "jpg",
        4_600_000,
        7,
        "nur_online",
      ),
      vdatei(
        "vd3",
        "Projektakten 2026/Angebote/Angebot Muster GmbH.pdf",
        "ord_od",
        "pdf",
        812_000,
        9,
        "ok",
      ),
      vdatei(
        "vd4",
        "Projektakten 2026/Angebote/Kostenschätzung.docx",
        "ord_od",
        "docx",
        96_000,
        11,
        "nur_online",
      ),
      vdatei(
        "vd5",
        "Projektakten 2026/Protokolle/Protokoll Begehung 14.10.md",
        "ord_od",
        "md",
        3_000,
        27,
        "ok",
      ),
      vdatei(
        "vd6",
        "Rechnungen 2025/Rechnung 2025-014.pdf",
        "ord_od",
        "pdf",
        210_000,
        40,
        "nur_online",
      ),
      vdatei(
        "vd7",
        "2019/Altakte Brücke Nord.pdf",
        "ord_arch",
        "pdf",
        640_000,
        400,
        "ok",
      ),
    ],
  };
}

const ANBIETER_EU = {
  regelwerk: "eu",
  anbieter: [
    {
      id: "beispiel_eu",
      name: "Beispiel-Cloud EU",
      waehlbar: true,
      grund: null,
    },
    { id: "muster_ki", name: "Muster-KI Europa", waehlbar: true, grund: null },
    {
      id: "demo_usa",
      name: "Demo-Anbieter USA",
      waehlbar: false,
      grund: "regelwerk",
    },
  ],
};
const ANBIETER_NUR_LOKAL = {
  regelwerk: "nur_lokal",
  anbieter: [
    {
      id: "beispiel_eu",
      name: "Beispiel-Cloud EU",
      waehlbar: false,
      grund: "regelwerk",
    },
  ],
};

interface MockOpt {
  theme: Theme;
  anbieter?: unknown;
  /** Ausnahmen, die schon bestehen: Ordner-Kennung → Anbieter-Kennungen. */
  ausnahmen?: Record<string, string[]>;
  /** Fehler der Seite nicht werfen (der Wechsel in die Einstellungen lädt dort andere Befehle). */
  fehlerDulden?: boolean;
}

async function installMock(page: Page, opts: MockOpt) {
  if (!opts.fehlerDulden)
    page.on("pageerror", (error) => {
      throw error;
    });
  await page.addInitScript(
    ({ sicht, theme, anbieter, ausnahmen }) => {
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
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as unknown[];
      w.__sicht = sicht;
      w.__anbieter = anbieter;
      window.localStorage.setItem("lva.ui.section", "ablage");
      const RANG: Record<string, number> = {
        public: 0,
        internal: 1,
        confidential: 2,
        personal: 3,
      };
      // Wie das Backend: wirksame Klasse = nie unter der des Elternordners; Ausnahmen gelten nur an
      // wirksam vertraulichen Ordnern (sonst entzogen); geerbte Ausnahmen kommen von oben.
      const neuBerechnen = () => {
        const s = w.__sicht;
        const je = new Map<string, any>(s.ordner.map((o: any) => [o.id, o]));
        const klasse = (o: any): string => {
          const eltern = o.eltern ? je.get(o.eltern) : null;
          const geerbt = eltern ? klasse(eltern) : "internal";
          const eigene = o.datenklasse === "erbt" ? geerbt : o.datenklasse;
          return RANG[eigene] >= RANG[geerbt] ? eigene : geerbt;
        };
        for (const o of s.ordner) o.klasse = klasse(o);
        for (const o of s.ordner)
          if (o.klasse !== "confidential") o.cloud.anbieter = [];
        const kette = (o: any) => {
          const liste: any[] = [];
          let c = o;
          while (c) {
            liste.push(c);
            c = c.eltern ? je.get(c.eltern) : null;
          }
          return liste;
        };
        // Die Cloud-Sicht gibt es nur an wirksam vertraulichen Einträgen (wie `cloud_ordner`).
        for (const o of s.ordner)
          o.cloud.geerbt =
            o.klasse !== "confidential"
              ? []
              : [
                  ...new Set(
                    kette(o)
                      .slice(1)
                      .flatMap((a: any) => a.cloud.anbieter),
                  ),
                ].sort();
        for (const d of s.dateien) {
          const o = je.get(d.ordner);
          d.klasse = o.klasse;
          d.cloud = {
            anbieter: [],
            geerbt:
              o.klasse !== "confidential"
                ? []
                : [
                    ...new Set(kette(o).flatMap((a: any) => a.cloud.anbieter)),
                  ].sort(),
          };
        }
      };
      for (const [id, liste] of Object.entries(ausnahmen ?? {})) {
        const o = sicht.ordner.find((x: any) => x.id === id);
        if (o) o.cloud.anbieter = [...(liste as string[])].sort();
      }
      neuBerechnen();
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
            const s = w.__sicht;
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.24.1";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return Number(args?.handler);
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "ablage_stand") return JSON.parse(JSON.stringify(s));
            if (cmd === "ablage_cloud_anbieter")
              return JSON.parse(JSON.stringify(w.__anbieter));
            if (cmd === "ablage_cloud_ausnahme_setzen") {
              const o = s.ordner.find((x: any) => x.id === args?.ordner);
              if (!o) throw "ablage.nicht_gefunden: Nicht gefunden.";
              const liste: string[] = o.cloud.anbieter;
              if (args?.an) {
                if (!liste.includes(args?.anbieter)) liste.push(args?.anbieter);
              } else
                o.cloud.anbieter = liste.filter((x) => x !== args?.anbieter);
              o.cloud.anbieter.sort();
              neuBerechnen();
              return null;
            }
            if (cmd === "ablage_ordner_einstellungen") {
              const o = s.ordner.find((x: any) => x.id === args?.id);
              const e = args?.einstellungen ?? {};
              if (e.datenklasse != null) o.datenklasse = e.datenklasse;
              if (e.nur_geraet != null) o.nur_geraet = e.nur_geraet;
              if (e.zielordner_ki != null) o.zielordner_ki = e.zielordner_ki;
              neuBerechnen();
              return null;
            }
            if (cmd === "ablage_ki_setzen") {
              const o = s.ordner.find((x: any) => x.id === args?.id);
              o.ki = {
                zustand: args?.stufe ?? "aus",
                von: null,
                von_name: null,
                vorgabe: args?.stufe == null,
              };
              return null;
            }
            if (cmd === "ablage_sync_stand")
              return {
                verbunden: false,
                letzter_lauf_ms: null,
                fehler: null,
                kontingent_voll: false,
                belegt: null,
                kontingent: null,
                eintraege: [],
              };
            if (cmd === "ablage_suchen") {
              const q = String(args?.anfrage ?? "").toLowerCase();
              if (!q.includes("angebot")) return [];
              return [
                {
                  dat: "vd3",
                  name: "Projektakten 2026/Angebote/Angebot Muster GmbH.pdf",
                  ordner: "ord_od",
                  typ: "pdf",
                  rang: 0.9,
                  stellen: [],
                },
              ];
            }
            if (cmd === "ablage_fassungen") return [];
            if (cmd.startsWith("ablage_semantik_"))
              return {
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
              };
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
      sicht: demoSicht(),
      theme: opts.theme,
      anbieter: opts.anbieter === undefined ? ANBIETER_EU : opts.anbieter,
      ausnahmen: opts.ausnahmen ?? {},
    },
  );
}

async function offne(page: Page, breite: number, theme: Theme) {
  await page.setViewportSize({ width: breite, height: HOEHE[breite] });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await expect(page.getByTestId("ablage-seite")).toBeVisible();
  await page.addStyleTag({
    content:
      "*,*::before,*::after{animation-duration:0s!important;animation-delay:0s!important;transition-duration:0s!important;transition-delay:0s!important}",
  });
}

const schmal = (breite: number) => breite < 1024;

/** Öffnet das Ordnerblatt (schmales Fenster) oder tut nichts (Spalte). Liefert den Baum. */
async function baum(page: Page, breite: number) {
  if (!schmal(breite)) return page.getByTestId("ablage-seite");
  const knopf = page.getByTestId("ablage-ordner-waehlen");
  if (!(await page.getByRole("dialog").count())) await knopf.click();
  return page.getByRole("dialog");
}

/** Wählt einen Ordner (oder Unterordner `<Kennung>|<Pfad>`) im Baum, klappt Vorfahren bei Bedarf auf. */
async function waehle(page: Page, breite: number, schluessel: string) {
  const wurzel = await baum(page, breite);
  const ziel = wurzel.getByTestId(`ablage-ordner-${schluessel}`);
  if (!(await ziel.isVisible())) {
    const [id, pfad = ""] = schluessel.split("|");
    const teile = pfad.split("/").filter(Boolean);
    const stufen = [
      id,
      ...teile
        .slice(0, -1)
        .map((_, i) => `${id}|${teile.slice(0, i + 1).join("/")}`),
    ];
    for (const st of stufen) {
      const u = wurzel.getByTestId(`ablage-baum-umschalter-${st}`);
      if (
        (await u.count()) &&
        (await u.getAttribute("aria-expanded")) === "false"
      )
        await u.click();
    }
  }
  await ziel.click();
  if (schmal(breite)) await expect(page.getByRole("dialog")).toHaveCount(0);
}

async function oeffneEinstellungen(page: Page) {
  await page.getByTestId("ablage-ordner-einstellungen").click();
  const dlg = page.getByRole("dialog");
  await expect(dlg).toBeVisible();
  return dlg;
}

async function aufrufe(page: Page) {
  return (await page.evaluate(
    () => (window as unknown as { __calls: [string, unknown][] }).__calls,
  )) as [string, Record<string, any> | undefined][];
}
const je = (a: [string, any][], cmd: string) =>
  a.filter(([c]) => c === cmd).map(([, args]) => args);

async function bedienflaechen(page: Page, wurzel: string) {
  return page.evaluate((root) => {
    const sel =
      "button, a[href], input:not([type=hidden]), select, textarea, [role=radio], [role=switch], [role=tab], [role=combobox], [role=checkbox]";
    const nodes = Array.from(document.querySelectorAll(root)).flatMap((r) =>
      Array.from(r.querySelectorAll(sel)),
    ) as HTMLElement[];
    const out: { name: string; w: number; h: number }[] = [];
    for (const el of new Set(nodes)) {
      const r = el.getBoundingClientRect();
      const cs = getComputedStyle(el);
      if (r.width === 0 || r.height === 0) continue;
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

// ===========================================================================================
// 1 Cloud-Ausnahme
// ===========================================================================================
test.describe("Cloud-Ausnahme im Ordnerdialog", () => {
  test("Abschnitt unter der Datenklasse: Schalter aus, Liste erst nach dem Einschalten", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    const abschnitt = dlg.getByTestId("ablage-cloud-abschnitt");
    await expect(abschnitt).toHaveText("Cloud-Ausnahme");
    const schalter = dlg.getByRole("switch", {
      name: "KI darf diesen Ordner über einen Cloud-Anbieter nutzen",
    });
    await expect(schalter).toHaveAttribute("aria-checked", "false");
    await expect(dlg.getByTestId("ablage-cloud-anbieter")).toHaveCount(0);
    // Reihenfolge: Datenklasse, dann Cloud-Ausnahme.
    const y = async (loc: ReturnType<Page["locator"]>) =>
      (await loc.boundingBox())!.y;
    expect(await y(dlg.getByText("Datenklasse", { exact: true }))).toBeLessThan(
      await y(abschnitt),
    );
    await schalter.click();
    const liste = dlg.getByTestId("ablage-cloud-anbieter");
    await expect(
      liste.getByText("Anbieter (Regelwerk „EU“)", { exact: true }),
    ).toBeVisible();
    await expect(dlg.getByTestId("ablage-cloud-info")).toContainText(
      "Unterordner mit „Personenbezogen“ sind ausgenommen",
    );
    // Gesperrter Anbieter: Kästchen deaktiviert, Text „gesperrt“ mit Grund.
    const usa = dlg.getByTestId("ablage-cloud-anbieter-demo_usa");
    await expect(usa).toBeDisabled();
    await expect(usa.locator("xpath=ancestor::label")).toContainText(
      "gesperrt · Erfüllt das Regelwerk „EU“ nicht.",
    );
    await expect(
      dlg.getByTestId("ablage-cloud-anbieter-beispiel_eu"),
    ).toBeEnabled();
    await expect(
      dlg.getByTestId("ablage-cloud-anbieter-muster_ki"),
    ).toBeEnabled();
  });

  test("Speichern ohne Anbieter: Hinweis, nichts wird gesendet", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }).click();
    await dlg.getByTestId("ablage-einstellungen-speichern").click();
    await expect(dlg.getByTestId("ablage-cloud-fehler")).toHaveText(
      "Wählen Sie mindestens einen Anbieter.",
    );
    await expect(dlg.getByTestId("ablage-cloud-bestaetigung")).toHaveCount(0);
    const a = await aufrufe(page);
    expect(je(a, "ablage_cloud_ausnahme_setzen")).toHaveLength(0);
    expect(je(a, "ablage_ordner_einstellungen")).toHaveLength(0);
  });

  test("Bestätigungsschritt: Frage als Titel, beide Wege, Zahl und Ausnahmen, Fokus auf „Zurück“; Erteilen setzt je Anbieter", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }).click();
    await dlg.getByTestId("ablage-cloud-anbieter-muster_ki").check();
    await dlg.getByTestId("ablage-cloud-anbieter-beispiel_eu").check();
    await dlg.getByTestId("ablage-einstellungen-speichern").click();

    // Die Frage ist der Dialogtitel (kein role=alert), die Zahl steht in der Beschreibung.
    const bestaetigung = page.getByRole("dialog", {
      name: "Cloud-Ausnahme für „Verträge“ erteilen?",
    });
    await expect(bestaetigung).toBeVisible();
    await expect(bestaetigung.getByRole("alert")).toHaveCount(0);
    // 5 = Dateien mit wirksamer Klasse „Vertraulich“ im Unterbaum; die Gehaltsliste (Personenbezogen) zählt nicht.
    await expect(bestaetigung).toHaveAccessibleDescription(
      /Auszüge aus 5 vertraulichen Dateien in „Verträge“ und seinen Unterordnern dürfen gesendet werden an:/,
    );
    const text = bestaetigung.getByTestId("ablage-cloud-bestaetigung");
    // Alle gewählten Anbieter, mit Regelwerk, sortiert nach Name.
    await expect(text.getByRole("listitem").nth(0)).toHaveText(
      "Beispiel-Cloud EU (Regelwerk „EU“)",
    );
    await expect(text.getByRole("listitem").nth(1)).toHaveText(
      "Muster-KI Europa (Regelwerk „EU“)",
    );
    await expect(text).toContainText(
      "Ausgenommen sind Unterordner mit der Datenklasse „Personenbezogen“: „Gehälter“. Sie gehen nie an einen Cloud-Anbieter.",
    );
    // M2: beide Wege.
    await expect(text).toContainText(
      "Die Ausnahme gilt für den Assistenten (KI-Freigabe „An“) und für Projekte, in denen der Ordner eingeschaltet ist. „Gesperrt“ schließt beides aus.",
    );
    await expect(text).toContainText("nur auf diesem Gerät");
    await expect(text).toContainText(
      "Bereits gesendete Auszüge lassen sich nicht zurückholen.",
    );
    // M1: Wortlaut ohne DSFA.
    await expect(text).toContainText(
      "Personenbezogene Daten gehören nicht in diesen Ordner. Stufen Sie sie als „Personenbezogen“ ein; sie gehen nie an einen Cloud-Anbieter.",
    );
    await expect(text).not.toContainText("Datenschutz-Folgenabschätzung");
    // Fokus auf der sicheren Wahl.
    await expect(
      bestaetigung.getByTestId("ablage-einstellungen-zurueck"),
    ).toBeFocused();
    // Noch nichts gesendet.
    expect(
      je(await aufrufe(page), "ablage_cloud_ausnahme_setzen"),
    ).toHaveLength(0);

    // Zurück: Formular mit der Auswahl.
    await bestaetigung.getByTestId("ablage-einstellungen-zurueck").click();
    await expect(
      dlg.getByTestId("ablage-cloud-anbieter-beispiel_eu"),
    ).toBeChecked();
    await expect(
      dlg.getByTestId("ablage-cloud-anbieter-muster_ki"),
    ).toBeChecked();
    await dlg.getByTestId("ablage-einstellungen-speichern").click();
    await page.getByTestId("ablage-cloud-erteilen").click();
    await expect(page.getByRole("dialog")).toHaveCount(0);

    const a = await aufrufe(page);
    expect(je(a, "ablage_cloud_ausnahme_setzen")).toEqual([
      { ordner: "o_vert", anbieter: "beispiel_eu", an: true },
      { ordner: "o_vert", anbieter: "muster_ki", an: true },
    ]);
    // Chip im Kopf als Knopf: Text „Cloud-Ausnahme“, aria-label nennt alle Anbieter.
    const chip = page.getByTestId("ablage-kopf-cloud");
    await expect(chip).toContainText("Cloud-Ausnahme");
    await expect(chip).toHaveAttribute(
      "aria-label",
      /Beispiel-Cloud EU, Muster-KI Europa/,
    );
    expect((await chip.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    await expect(page.getByTestId("ablage-baum-cloud-o_vert")).toHaveAttribute(
      "aria-label",
      "Cloud-Ausnahme: Beispiel-Cloud EU, Muster-KI Europa",
    );
    // Unterordner: Chip „geerbt“ in der Liste.
    const wart = page
      .getByTestId("ablage-unterordner-o_wart")
      .locator("xpath=ancestor::tr");
    await expect(wart).toContainText("Cloud-Ausnahme (geerbt)");
    // Der Chip öffnet die Ordnereinstellungen.
    await chip.click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(
      page
        .getByRole("dialog")
        .getByRole("switch", { name: /Cloud-Anbieter nutzen/ }),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("Details einer Datei nennen Anbieter und den Ordner, von dem die Ausnahme stammt", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      ausnahmen: { o_vert: ["beispiel_eu"] },
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_wart");
    await page.getByTestId("ablage-datei-d6").click();
    const zeile = page
      .getByTestId("ablage-seite-details")
      .getByTestId("ablage-details-cloud");
    await expect(zeile).toContainText(
      "Beispiel-Cloud EU · geerbt von Verträge",
    );
  });

  test("Entziehen braucht keine Bestätigung; der Chip verschwindet", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      ausnahmen: { o_vert: ["beispiel_eu", "muster_ki"] },
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    await expect(page.getByTestId("ablage-kopf-cloud")).toBeVisible();
    const dlg = await oeffneEinstellungen(page);
    const schalter = dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ });
    await expect(schalter).toHaveAttribute("aria-checked", "true");
    await schalter.click();
    await dlg.getByTestId("ablage-einstellungen-speichern").click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const a = await aufrufe(page);
    expect(je(a, "ablage_cloud_ausnahme_setzen")).toEqual([
      { ordner: "o_vert", anbieter: "beispiel_eu", an: false },
      { ordner: "o_vert", anbieter: "muster_ki", an: false },
    ]);
    await expect(page.getByTestId("ablage-kopf-cloud")).toHaveCount(0);
  });

  test("Datenklasse weg von „Vertraulich“: Schalter entfällt, Hinweis, Ausnahme verfällt beim Speichern", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      ausnahmen: { o_vert: ["beispiel_eu"] },
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await dlg.getByRole("combobox", { name: "Datenklasse" }).click();
    await page.getByRole("option", { name: "Intern", exact: true }).click();
    await expect(
      dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }),
    ).toHaveCount(0);
    await expect(dlg.getByTestId("ablage-cloud-braucht-keine")).toContainText(
      "Für „Intern“ braucht es keine Ausnahme",
    );
    await expect(dlg.getByTestId("ablage-cloud-verfaellt")).toContainText(
      "Die Cloud-Ausnahme (Beispiel-Cloud EU) verfällt mit dem Speichern.",
    );
    await dlg.getByTestId("ablage-einstellungen-speichern").click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const a = await aufrufe(page);
    expect(
      je(a, "ablage_ordner_einstellungen")[0]?.einstellungen.datenklasse,
    ).toBe("internal");
    // Das Backend entzieht; die Oberfläche schickt keinen zweiten Entzug.
    expect(je(a, "ablage_cloud_ausnahme_setzen")).toHaveLength(0);
    await expect(page.getByTestId("ablage-kopf-cloud")).toHaveCount(0);
  });

  test("Zuerst „Vertraulich“ speichern: vor dem Speichern gibt es keinen Schalter", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_allg");
    const dlg = await oeffneEinstellungen(page);
    await expect(dlg.getByTestId("ablage-cloud-braucht-keine")).toContainText(
      "Für „Intern“ braucht es keine Ausnahme",
    );
    await dlg.getByRole("combobox", { name: "Datenklasse" }).click();
    await page
      .getByRole("option", { name: "Vertraulich", exact: true })
      .click();
    await expect(dlg.getByTestId("ablage-cloud-erst-speichern")).toContainText(
      "Speichern Sie zuerst die Datenklasse „Vertraulich“.",
    );
    await expect(
      dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }),
    ).toHaveCount(0);
  });

  test("Personenbezogen: nie Cloud, kein Schalter", async ({ page }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_gehalt");
    const dlg = await oeffneEinstellungen(page);
    await expect(dlg.getByTestId("ablage-cloud-braucht-keine")).toContainText(
      "Eine Cloud-Ausnahme gibt es dafür nicht.",
    );
    await expect(
      dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }),
    ).toHaveCount(0);
  });

  test("KI „Gesperrt“: Erteilen gesperrt mit Hinweis, eine bestehende Ausnahme bleibt entziehbar", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    let dlg = await oeffneEinstellungen(page);
    await dlg.getByTestId("ablage-ki-gesperrt").click();
    const schalter = dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ });
    await expect(schalter).toBeDisabled();
    await expect(dlg.getByTestId("ablage-cloud-sperre")).toContainText(
      "Die KI-Freigabe ist „Gesperrt“.",
    );
  });

  test("KI „Gesperrt“ bei bestehender Ausnahme: Schalter bleibt bedienbar (Entziehen)", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      ausnahmen: { o_vert: ["beispiel_eu"] },
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await dlg.getByTestId("ablage-ki-gesperrt").click();
    const schalter = dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ });
    await expect(schalter).toBeEnabled();
    await expect(schalter).toHaveAttribute("aria-checked", "true");
    await expect(dlg.getByTestId("ablage-cloud-sperre")).toBeVisible();
    await schalter.click();
    await expect(schalter).toHaveAttribute("aria-checked", "false");
    // Wieder anschalten geht bei „Gesperrt“ nicht mehr.
    await expect(schalter).toBeDisabled();
  });

  test("Regelwerk „Nur lokal“: Schalter gesperrt, Regelwerk genannt, Link und Warnung vor ungespeicherten Änderungen", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      anbieter: ANBIETER_NUR_LOKAL,
      fehlerDulden: true,
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    const schalter = dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ });
    await expect(schalter).toBeDisabled();
    await expect(dlg.getByTestId("ablage-cloud-sperre")).toContainText(
      "Das Regelwerk „Nur lokal“ lässt keine Cloud-Anbieter zu.",
    );
    const link = dlg.getByTestId("ablage-cloud-regelwerk");
    await expect(link).toHaveText("Regelwerk und Anbieter verwalten");
    expect((await link.boundingBox())!.height).toBeGreaterThanOrEqual(44);

    // S8: mit ungespeicherter Eingabe nicht einfach verlassen.
    await dlg.getByTestId("ablage-ki-aus").click();
    await link.click();
    const frage = page.getByRole("dialog", {
      name: "Ungespeicherte Änderungen verwerfen?",
    });
    await expect(frage).toBeVisible();
    await expect(frage).toHaveAccessibleDescription(
      /Wenn Sie jetzt zu „KI-Modelle & Anbieter“ wechseln/,
    );
    await expect(
      frage.getByTestId("ablage-einstellungen-zurueck"),
    ).toBeFocused();
    await frage.getByTestId("ablage-einstellungen-zurueck").click();
    await expect(dlg.getByTestId("ablage-cloud-regelwerk")).toBeVisible();
    // Zurücknehmen: ohne Änderung geht es direkt.
    await dlg.getByTestId("ablage-ki-an").click();
    await dlg.getByTestId("ablage-cloud-regelwerk").click();
    await expect(
      page.getByRole("dialog", {
        name: "Ungespeicherte Änderungen verwerfen?",
      }),
    ).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => window.localStorage.getItem("lva.ui.settings.tab")),
      )
      .toBe("models");
  });

  test("Verwerfen und wechseln: der Dialog schließt, die Einstellungen „KI-Modelle & Anbieter“ öffnen sich", async ({
    page,
  }) => {
    await installMock(page, {
      theme: "light",
      anbieter: ANBIETER_NUR_LOKAL,
      fehlerDulden: true,
    });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await dlg.getByTestId("ablage-ki-aus").click();
    await dlg.getByTestId("ablage-cloud-regelwerk").click();
    await page.getByTestId("ablage-cloud-verlassen").click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => window.localStorage.getItem("lva.ui.settings.tab")),
      )
      .toBe("models");
    // Gespeichert wurde nichts.
    const a = await aufrufe(page);
    expect(je(a, "ablage_ki_setzen")).toHaveLength(0);
  });

  test("Anbieterliste nicht ladbar: Hinweis, Schalter gesperrt, nichts geht verloren", async ({
    page,
  }) => {
    await installMock(page, { theme: "light", anbieter: null });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "o_vert");
    const dlg = await oeffneEinstellungen(page);
    await expect(dlg.getByTestId("ablage-cloud-nicht-geladen")).toContainText(
      "Die Anbieterliste ließ sich nicht laden.",
    );
    await expect(
      dlg.getByRole("switch", { name: /Cloud-Anbieter nutzen/ }),
    ).toBeDisabled();
  });

  test("Verknüpfter Ordner: keine KI-Freigabe, keine Cloud-Ausnahme, Erklärung", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "ord_od");
    const dlg = await oeffneEinstellungen(page);
    await expect(dlg.getByTestId("ablage-einstellungen-chip")).toHaveText(
      "Verknüpft – nur lesen, nur dieses Gerät",
    );
    await expect(dlg.getByTestId("ablage-ki-nicht-verfuegbar")).toBeVisible();
    await expect(dlg.getByTestId("ablage-keine-ki")).toContainText(
      "Die KI greift auf verknüpfte Ordner nie zu – weder mit lokalen Modellen noch über einen Cloud-Anbieter.",
    );
    await expect(dlg.getByTestId("ablage-cloud-abschnitt")).toHaveCount(0);
    await expect(dlg.getByRole("switch")).toHaveCount(0);
    await expect(dlg).toContainText("Datenklasse");
  });
});

// ===========================================================================================
// 2 Unterordnerbaum verknüpfter Ordner
// ===========================================================================================
test.describe("Unterordnerbaum verknüpfter Ordner", () => {
  for (const breite of [1440, 390] as const) {
    test(`${breite} px: aufklappbar, nur lesen erkennbar, Unterordner öffnet seine Liste`, async ({
      page,
    }) => {
      await installMock(page, { theme: "light" });
      await offne(page, breite, "light");
      const b = await baum(page, breite);
      await expect(b.getByTestId("ablage-verknuepft-gruppe")).toHaveText(
        "Verknüpfte Ordner (nur lesen)",
      );
      const wurzel = b.getByTestId("ablage-ordner-ord_od");
      await expect(wurzel).toBeVisible();
      // Zu: kein Unterordner sichtbar; der Knopf nennt den Zustand.
      const u = b.getByTestId("ablage-baum-umschalter-ord_od");
      await expect(u).toHaveAttribute("aria-expanded", "false");
      await expect(u).toHaveAccessibleName("OneDrive – Dokumente aufklappen");
      expect((await u.boundingBox())!.width).toBeGreaterThanOrEqual(44);
      expect((await u.boundingBox())!.height).toBeGreaterThanOrEqual(44);
      await expect(
        b.getByTestId("ablage-ordner-ord_od|Projektakten 2026"),
      ).toBeHidden();
      await u.click();
      await expect(u).toHaveAttribute("aria-expanded", "true");
      await expect(u).toHaveAccessibleName("OneDrive – Dokumente zuklappen");
      await expect(
        b.getByTestId("ablage-ordner-ord_od|Projektakten 2026"),
      ).toBeVisible();
      // Zweite Ebene.
      await b
        .getByTestId("ablage-baum-umschalter-ord_od|Projektakten 2026")
        .click();
      await expect(
        b.getByTestId("ablage-ordner-ord_od|Projektakten 2026/Angebote"),
      ).toBeVisible();
      // Nur lesen: Symbol mit Text, Auge am Ordner.
      await expect(
        wurzel.getByRole("img", { name: "Nur lesen" }),
      ).toBeVisible();
      // „Nur online“-Unterordner: Symbol mit Text, nicht aufklappbar.
      const online = b.getByTestId("ablage-ordner-ord_od|Rechnungen 2025");
      await expect(
        online.getByRole("img", { name: "Nur online" }),
      ).toBeVisible();
      await expect(
        b.getByTestId("ablage-baum-umschalter-ord_od|Rechnungen 2025"),
      ).toHaveCount(0);

      // Auswahl: Liste und Kopf des Unterordners.
      await b
        .getByTestId("ablage-ordner-ord_od|Projektakten 2026/Angebote")
        .click();
      if (schmal(breite)) await expect(page.getByRole("dialog")).toHaveCount(0);
      await expect(page.getByTestId("ablage-ordnername")).toHaveText(
        "Angebote",
      );
      await expect(page.getByTestId("ablage-kopf-nur-lesen")).toHaveText(
        "Nur lesen",
      );
      await expect(page.getByTestId("ablage-datei-vd3")).toBeVisible();
      await expect(page.getByTestId("ablage-datei-vd4")).toBeVisible();
      // Nur, was direkt in diesem Ordner liegt.
      await expect(page.getByTestId("ablage-datei-vd5")).toHaveCount(0);
      await expect(page.getByTestId("ablage-datei-vd1")).toHaveCount(0);
      // Dateiname ohne Pfad, der Pfad steht im Titel.
      await expect(page.getByTestId("ablage-datei-vd3")).toContainText(
        "Angebot Muster GmbH.pdf",
      );
      await expect(page.getByTestId("ablage-datei-vd3")).not.toContainText(
        "Projektakten",
      );
      // Zurück geht eine Ebene hoch.
      await page.getByTestId("ablage-zurueck").click();
      await expect(page.getByTestId("ablage-ordnername")).toHaveText(
        "Projektakten 2026",
      );
      await expect(
        page.getByTestId(
          "ablage-unterordner-ord_od|Projektakten 2026/Angebote",
        ),
      ).toBeVisible();
      await expect(
        page.getByTestId(
          "ablage-unterordner-ord_od|Projektakten 2026/Protokolle",
        ),
      ).toBeVisible();
      await page.getByTestId("ablage-zurueck").click();
      await expect(page.getByTestId("ablage-ordnername")).toHaveText(
        "OneDrive – Dokumente",
      );
      await expect(page.getByTestId("ablage-zurueck")).toHaveCount(0);
      await breiteOk(page);
    });
  }

  test("Tastatur: Umschalter per Eingabetaste und Leertaste, Reihenfolge Umschalter dann Ordner", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    const u = page.getByTestId("ablage-baum-umschalter-ord_od");
    await u.focus();
    await page.keyboard.press("Enter");
    await expect(u).toHaveAttribute("aria-expanded", "true");
    await page.keyboard.press("Space");
    await expect(u).toHaveAttribute("aria-expanded", "false");
    await page.keyboard.press("Space");
    await page.keyboard.press("Tab");
    await expect(page.getByTestId("ablage-ordner-ord_od")).toBeFocused();
  });

  test("Nicht erreichbar: Warnzeichen, gedämpfter Name, zuletzt bekannter Baum, „Erneut prüfen“ und Lösen", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await page.getByTestId("ablage-baum-umschalter-ord_arch").click();
    await expect(
      page
        .getByTestId("ablage-ordner-ord_arch")
        .getByRole("img", { name: "Nicht erreichbar" }),
    ).toBeVisible();
    await expect(page.getByTestId("ablage-ordner-ord_arch|2019")).toBeVisible();
    await page.getByTestId("ablage-ordner-ord_arch").click();
    const meldung = page.getByTestId("ablage-verknuepft-unerreichbar");
    await expect(meldung).toContainText(
      "Der verknüpfte Ordner ist gerade nicht erreichbar",
    );
    await expect(
      page.getByTestId("ablage-unterordner-ord_arch|2019"),
    ).toBeVisible();
    // Zuletzt bekannte Dateien in der Unterordner-Ansicht.
    await page.getByTestId("ablage-unterordner-ord_arch|2019").click();
    await expect(page.getByTestId("ablage-zuletzt-vd7")).toHaveText(
      "Zuletzt bekannt",
    );
    await page.getByTestId("ablage-zurueck").click();
    // „Erneut prüfen“ liest neu ein, „Verknüpfung lösen“ fragt nach.
    await page.getByTestId("ablage-unerreichbar-einlesen").click();
    await expect
      .poll(async () => je(await aufrufe(page), "ablage_verknuepft_abtasten"))
      .toEqual([{ id: "ord_arch" }]);
    await page.getByTestId("ablage-unerreichbar-loesen").click();
    await expect(page.getByRole("dialog")).toContainText(
      "nicht mehr verknüpfen?",
    );
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });

  test("Suche öffnet die Datei im Unterordner und klappt den Baum bis dorthin auf", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await page
      .getByPlaceholder("Namen und Inhalte durchsuchen")
      .fill("angebot");
    const treffer = page
      .getByRole("button", { name: /Angebot Muster GmbH\.pdf/ })
      .first();
    await treffer.click();
    await expect(page.getByTestId("ablage-ordnername")).toHaveText("Angebote");
    await expect(
      page.getByTestId("ablage-ordner-ord_od|Projektakten 2026/Angebote"),
    ).toBeVisible();
    await expect(
      page.getByTestId("ablage-baum-umschalter-ord_od|Projektakten 2026"),
    ).toHaveAttribute("aria-expanded", "true");
    await expect(page.getByTestId("ablage-seite-details")).toContainText(
      "Angebot Muster GmbH.pdf",
    );
  });
});

// ===========================================================================================
// 3 „Nur online“
// ===========================================================================================
test.describe("Nur online (OneDrive-Platzhalter)", () => {
  test("Stand in der Liste: Chip mit Text und Hinweis bei Hover und Fokus, Escape schließt", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "ord_od");
    const chip = page.getByTestId("ablage-nur-online-vd2");
    await expect(chip).toContainText("Nur online");
    expect((await chip.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    // Lesbare Dateien: „Im Index“ mit Text.
    await expect(
      page.getByTestId("ablage-datei-vd1").locator("xpath=ancestor::tr"),
    ).toContainText("Im Index");
    const tipp = page.getByTestId("ablage-nur-online-vd2-tipp");
    await expect(tipp).toBeHidden();
    await chip.hover();
    await expect(tipp).toBeVisible();
    await expect(tipp).toHaveAttribute("role", "tooltip");
    await expect(tipp).toContainText(
      "liegt nur online beim Cloud-Anbieter (z. B. OneDrive)",
    );
    await expect(tipp).toContainText("Immer auf diesem Gerät behalten");
    await expect(chip).toHaveAttribute(
      "aria-describedby",
      (await tipp.getAttribute("id"))!,
    );
    // Escape schließt, ohne die Seite zu verlassen (WCAG 1.4.13).
    await chip.focus();
    await page.keyboard.press("Escape");
    await expect(tipp).toBeHidden();
    await expect(page.getByTestId("ablage-seite")).toBeVisible();
    // Fokus zeigt ihn wieder.
    await page.mouse.move(0, 0);
    await page.getByTestId("ablage-datei-vd1").focus();
    await page.keyboard.press("Tab");
    await chip.focus();
    await expect(tipp).toBeVisible();
  });

  test("Hinweis „Auf diesem Gerät behalten“: wörtlicher Windows-Schritt, kein Download, kein Explorer-Knopf; Neu einlesen", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "ord_od");
    await page.getByTestId("ablage-nur-online-vd2").click();
    const details = page.getByTestId("ablage-seite-details");
    const hinweis = details.getByTestId("ablage-nur-online-hinweis");
    await expect(hinweis).toContainText("Auf diesem Gerät behalten");
    await expect(hinweis).toContainText("Immer auf diesem Gerät behalten");
    await expect(hinweis).toContainText(
      "in der Regel innerhalb einer Minute selbst ein",
    );
    await expect(hinweis).not.toContainText("Im Explorer zeigen");
    await expect(
      details.getByRole("button", { name: /Explorer|herunterladen|Download/i }),
    ).toHaveCount(0);
    await expect(details).toContainText("Keine Vorschau");
    await details.getByTestId("ablage-nur-online-einlesen").click();
    const a = await aufrufe(page);
    expect(je(a, "ablage_verknuepft_abtasten")).toEqual([{ id: "ord_od" }]);
    // Nie hochladen, nie herunterladen: außer dem Einlesen geht nichts an die Dateien.
    const schreibend = a.filter(
      ([c]) =>
        c.startsWith("ablage_") &&
        (/herunterladen|download|importieren|hochladen|verschieben/i.test(c) ||
          c === "ablage_cloud_ausnahme_setzen"),
    );
    expect(schreibend).toHaveLength(0);
  });

  test("S3: Zählung nennt „beim Cloud-Anbieter (z. B. OneDrive)“", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 1440, "light");
    await waehle(page, 1440, "ord_od");
    await expect(
      page.getByTestId("ablage-verknuepft-nur-online"),
    ).toContainText(
      "3 Einträge liegen nur online beim Cloud-Anbieter (z. B. OneDrive) und werden nicht gelesen.",
    );
  });

  test("390 px: Karten statt Tabelle nur im verknüpften Ordner, Eigene Ordner bleiben Tabelle", async ({
    page,
  }) => {
    await installMock(page, { theme: "light" });
    await offne(page, 390, "light");
    await waehle(page, 390, "ord_od");
    const tabelle = page.getByRole("table");
    await expect(tabelle).toHaveClass(/ablage-karten/);
    const zeile = page
      .getByTestId("ablage-datei-vd2")
      .locator("xpath=ancestor::tr");
    await expect(zeile).toContainText("JPG");
    await expect(zeile.getByTestId("ablage-nur-online-vd2")).toBeVisible();
    await breiteOk(page);
    await waehle(page, 390, "o_allg");
    await expect(page.getByRole("table")).not.toHaveClass(/ablage-karten/);
  });
});

// ===========================================================================================
// Aufnahmen und Maße: hell/dunkel × 1440/768/390
// ===========================================================================================
interface Befund {
  bild: string;
  scrollBreit: number;
  fenster: number;
  unter44: string[];
}
const befunde: Befund[] = [];

async function aufnahme(
  page: Page,
  screen: string,
  breite: number,
  theme: Theme,
  wurzel: string,
) {
  const bild = `${screen}-${breite}-${NAME_THEME[theme]}.png`;
  const m = await breiteOk(page);
  const flaechen = schmal(breite) ? await bedienflaechen(page, wurzel) : [];
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

for (const theme of THEMEN) {
  for (const breite of BREITEN) {
    test.describe(`Aufnahmen ${breite} px ${NAME_THEME[theme]}`, () => {
      const DLG = '[role="dialog"]';

      test("Cloud-Ausnahme: aus, Anbieter, Bestätigung, Chip, Klassenwechsel, verknüpft", async ({
        page,
      }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        await waehle(page, breite, "o_vert");
        let dlg = await oeffneEinstellungen(page);
        await expect(dlg.getByTestId("ablage-cloud-abschnitt")).toBeVisible();
        await dlg
          .getByTestId("ablage-cloud-abschnitt")
          .scrollIntoViewIfNeeded();
        await aufnahme(page, "01-cloud-aus", breite, theme, DLG);
        await dlg
          .getByRole("switch", { name: /Cloud-Anbieter nutzen/ })
          .click();
        await dlg.getByTestId("ablage-cloud-anbieter-beispiel_eu").check();
        await dlg.getByTestId("ablage-cloud-anbieter").scrollIntoViewIfNeeded();
        await aufnahme(page, "02-cloud-anbieter", breite, theme, DLG);
        await dlg.getByTestId("ablage-einstellungen-speichern").click();
        await expect(page.getByTestId("ablage-cloud-erteilen")).toBeVisible();
        await aufnahme(page, "03-cloud-bestaetigen", breite, theme, DLG);
        await page.getByTestId("ablage-cloud-erteilen").click();
        await expect(page.getByRole("dialog")).toHaveCount(0);
        await expect(page.getByTestId("ablage-kopf-cloud")).toBeVisible();
        await aufnahme(
          page,
          "04-cloud-chip",
          breite,
          theme,
          '[data-testid="ablage-seite"]',
        );
        dlg = await oeffneEinstellungen(page);
        await dlg.getByRole("combobox", { name: "Datenklasse" }).click();
        await page.getByRole("option", { name: "Intern", exact: true }).click();
        await expect(dlg.getByTestId("ablage-cloud-verfaellt")).toBeVisible();
        await dlg
          .getByTestId("ablage-cloud-verfaellt")
          .scrollIntoViewIfNeeded();
        await aufnahme(page, "05-cloud-klassenwechsel", breite, theme, DLG);
        // Nicht Escape: das Auswahlfeld nimmt die Taste auch bei geschlossener Liste an.
        await dlg.getByRole("button", { name: "Abbrechen" }).click();
        await expect(page.getByRole("dialog")).toHaveCount(0);
        await waehle(page, breite, "ord_od");
        dlg = await oeffneEinstellungen(page);
        await expect(dlg.getByTestId("ablage-keine-ki")).toBeVisible();
        await aufnahme(page, "06-verknuepft-eigenschaften", breite, theme, DLG);
      });

      test("Baum, nicht erreichbar, nur online", async ({ page }) => {
        await installMock(page, { theme });
        await offne(page, breite, theme);
        // Baum aufgeklappt.
        await waehle(page, breite, "ord_od|Projektakten 2026/Angebote");
        if (schmal(breite)) {
          await page.getByTestId("ablage-ordner-waehlen").click();
          await expect(page.getByRole("dialog")).toBeVisible();
          await aufnahme(page, "07-baum", breite, theme, DLG);
          await page.keyboard.press("Escape");
          await expect(page.getByRole("dialog")).toHaveCount(0);
        } else {
          await aufnahme(
            page,
            "07-baum",
            breite,
            theme,
            '[data-testid="ablage-seite"]',
          );
        }
        // Nicht erreichbar.
        const b = await baum(page, breite);
        const u = b.getByTestId("ablage-baum-umschalter-ord_arch");
        if ((await u.getAttribute("aria-expanded")) === "false")
          await u.click();
        if (schmal(breite)) {
          await aufnahme(
            page,
            "08-baum-nicht-erreichbar-blatt",
            breite,
            theme,
            DLG,
          );
          await b.getByTestId("ablage-ordner-ord_arch").click();
        } else {
          await b.getByTestId("ablage-ordner-ord_arch").click();
        }
        await expect(
          page.getByTestId("ablage-verknuepft-unerreichbar"),
        ).toBeVisible();
        await aufnahme(
          page,
          "08-baum-nicht-erreichbar",
          breite,
          theme,
          '[data-testid="ablage-seite"]',
        );
        // Nur online: Liste, Hinweis (Tooltip nur mit Zeiger).
        await waehle(page, breite, "ord_od");
        await expect(page.getByTestId("ablage-nur-online-vd2")).toBeVisible();
        await aufnahme(
          page,
          "09-nur-online-liste",
          breite,
          theme,
          '[data-testid="ablage-seite"]',
        );
        if (!schmal(breite) || breite === 768) {
          await page.getByTestId("ablage-nur-online-vd2").hover();
          await expect(
            page.getByTestId("ablage-nur-online-vd2-tipp"),
          ).toBeVisible();
          await aufnahme(
            page,
            "09b-nur-online-tooltip",
            breite,
            theme,
            '[data-testid="ablage-seite"]',
          );
          await page.mouse.move(0, 0);
        }
        await page.getByTestId("ablage-nur-online-vd2").click();
        await expect(
          page.getByTestId("ablage-nur-online-hinweis"),
        ).toBeVisible();
        await aufnahme(
          page,
          "10-nur-online-hinweis",
          breite,
          theme,
          schmal(breite) ? DLG : '[data-testid="ablage-seite"]',
        );
      });
    });
  }
}
