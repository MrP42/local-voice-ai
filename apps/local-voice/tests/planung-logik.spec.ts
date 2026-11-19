import { test, expect } from "@playwright/test";
import * as fs from "node:fs";
import * as path from "node:path";
import type { Consent, Plan, PlanLink } from "../src/bindings";
import {
  BEKANNTE_FEHLER,
  REGEL_CODES,
  agendaSumme,
  beginnAusText,
  datumIsoAusText,
  datumTextAusIso,
  eingabeAusFormular,
  einwilligungenSortiert,
  fehlerSchluessel,
  formularAusPlan,
  formularGueltig,
  klassifizierungen,
  leeresFormular,
  listenGruppe,
  minutenAusText,
  regelParameter,
  regelSchluessel,
  sortiere,
  statusMarken,
  uebergabeErlaubt,
} from "../src/components/planung/logik";

// Besprechungsplanung P2 (lva#534): reine Logik der Planungsseite und die Texte in Deutsch und
// Englisch. Kein Browser, keine Tauri-Attrappe, nur synthetische Daten (example.org).

const wurzel = process.cwd();
const locale = (lang: string) =>
  JSON.parse(
    fs.readFileSync(
      path.resolve(wurzel, `src/i18n/locales/${lang}/translation.json`),
      "utf-8",
    ),
  );
const blatt = (lang: string, ...pfad: string[]) =>
  pfad.reduce<any>((n, p) => n?.[p], locale(lang));

const plan = (extra: Partial<Plan> = {}): Plan => ({
  id: "01HPLAN",
  meeting_id: null,
  title: "Abstimmung Pilotstart",
  goal_type: "entscheiden",
  goal_text: "Wir entscheiden über den Pilotstart.",
  expected_result: "Entscheidung mit Termin",
  duration_min: 25,
  classification: "intern",
  agenda: [
    { minutes: 5, topic: "Stand", owner: "A. Beispiel" },
    { minutes: 15, topic: "Optionen", owner: null },
  ],
  roles: {
    moderation: "A. Beispiel",
    ergebnissicherung: null,
    entscheidung: null,
  },
  materials: [
    {
      label: "Vorlage",
      url: "https://intranet.example.org/vorlage",
      due_date: "2026-11-20",
    },
  ],
  hints: {
    weiterleiten_erlaubt: false,
    barrierefreiheit: null,
    ergebnisfrist: "2026-11-30",
    ablageort: null,
  },
  transcription_planned: true,
  minutes_template_id: null,
  status: "entwurf",
  created_at: 1,
  updated_at: 1,
  ...extra,
});

const link = (extra: Partial<PlanLink> = {}): PlanLink => ({
  plan_id: "01HPLAN",
  adapter: "ics_marker",
  source_id: null,
  remote_id: null,
  ical_uid: "uid-1@example.org",
  change_key: null,
  starts_at: 1_793_000_000_000,
  ends_at: 1_793_001_500_000,
  location: null,
  join_url: null,
  body_hash: null,
  last_seen_at: null,
  last_change_at: null,
  sync_state: "ok",
  verschiebung_offen: false,
  block_konflikt: false,
  abgesagt: false,
  instanz: null,
  fremd_beobachtet: false,
  ...extra,
});

test("Formular und Eingabe: Rundlauf, Leerzeilen entfallen, Leerwerte werden null", () => {
  const p = plan();
  const form = formularAusPlan(p);
  const ein = eingabeAusFormular(form);
  expect(ein.title).toBe(p.title);
  expect(ein.agenda).toEqual(p.agenda);
  expect(ein.roles).toEqual(p.roles);
  expect(ein.materials).toEqual(p.materials);
  expect(ein.hints).toEqual(p.hints);
  expect(ein.duration_min).toBe(25);

  const leer = leeresFormular();
  leer.title = "  Titel  ";
  leer.agenda = [
    { topic: "", minutes: "", owner: "" }, // eben angelegt, nie ausgefüllt
    { topic: "Thema", minutes: "abc", owner: "" }, // Zeitbox ungültig
    { topic: "", minutes: "10", owner: "B. Muster" }, // ohne Thema: bleibt, Regelfehler im Backend
  ];
  leer.materials = [
    { label: "", url: "", due_date: "" },
    { label: "", url: "https://example.org/x", due_date: "" },
  ];
  const e2 = eingabeAusFormular(leer);
  expect(e2.title).toBe("Titel");
  expect(e2.agenda).toEqual([
    { topic: "Thema", minutes: 0, owner: null },
    { topic: "", minutes: 10, owner: "B. Muster" },
  ]);
  expect(e2.materials).toEqual([
    {
      label: "https://example.org/x",
      url: "https://example.org/x",
      due_date: null,
    },
  ]);
  expect(e2.expected_result).toBeNull();
  expect(e2.hints?.weiterleiten_erlaubt).toBeNull();
  expect(e2.classification).toBe("intern");
  expect(agendaSumme(leer)).toBe(10);
});

test("Zahlen, Uhrzeit und Klassifizierung", () => {
  expect(minutenAusText("25")).toBe(25);
  for (const s of ["", " ", "-5", "2.5", "1e3", "abc", "9999999"])
    expect(minutenAusText(s), s).toBe(0);
  expect(beginnAusText("09:30")).toEqual({ stunde: 9, minute: 30 });
  for (const s of ["", "9:05", "25:00", "24:00", "12:60", "zwölf", "12"])
    expect(beginnAusText(s), s).toBeNull();
  expect(klassifizierungen("intern")).toContain("intern");
  expect(klassifizierungen("Sonderfall").at(-1)).toBe("Sonderfall");
  expect(klassifizierungen("").length).toBe(4);
});

test("Protokollvorlage bleibt beim Ändern des Titels erhalten", () => {
  const form = formularAusPlan(plan({ minutes_template_id: "vorlage-1" }));
  form.title = "Neuer Titel";
  expect(eingabeAusFormular(form).minutes_template_id).toBe("vorlage-1");
});

test("Agenda ohne Thema: Zeitbox oder Verantwortlicher bleiben für die Regelprüfung", () => {
  const form = leeresFormular();
  form.agenda = [
    { topic: "", minutes: "", owner: "" },
    { topic: "", minutes: "5", owner: "" },
    { topic: "", minutes: "abc", owner: "" },
    { topic: "", minutes: "", owner: "A. Beispiel" },
  ];
  expect(eingabeAusFormular(form).agenda).toEqual([
    { topic: "", minutes: 5, owner: null },
    { topic: "", minutes: 0, owner: null },
    { topic: "", minutes: 0, owner: "A. Beispiel" },
  ]);
});

test("Datumseingabe und Uhrzeit: leer ist optional, ungültig sperrt", () => {
  expect(datumTextAusIso("2026-11-20")).toBe("20.11.2026");
  expect(datumIsoAusText("20.11.2026")).toBe("2026-11-20");
  expect(datumIsoAusText("29.02.2024")).toBe("2024-02-29");
  for (const s of ["31.02.2026", "29.02.2025", "2026-11-20", "1.1.2026"])
    expect(datumIsoAusText(s)).toBeNull();
  const form = leeresFormular();
  expect(formularGueltig(form)).toBe(true);
  form.beginn = "25:00";
  expect(formularGueltig(form)).toBe(false);
  form.beginn = "09:00";
  expect(formularGueltig(form)).toBe(true);
  form.ergebnisfrist = "31.02.2026";
  expect(formularGueltig(form)).toBe(false);
});

test("Statusmarken: verknüpft, verschoben, abgesagt, Konflikt, jeweils nur einmal", () => {
  const schluessel = (p: Plan, l: PlanLink | null) =>
    statusMarken(p, l).map((m) => m.schluessel);
  expect(schluessel(plan(), null)).toEqual(["entwurf"]);
  expect(
    schluessel(plan({ status: "uebergeben" }), link({ starts_at: null })),
  ).toEqual(["uebergeben"]);
  expect(schluessel(plan({ status: "verknuepft" }), link())).toEqual([
    "verknuepft",
  ]);
  // Status und Merker melden dasselbe: eine Marke.
  expect(
    schluessel(
      plan({ status: "verschoben" }),
      link({ sync_state: "verschoben", verschiebung_offen: true }),
    ),
  ).toEqual(["verschoben"]);
  // Der Merker lebt unter einem Konflikt weiter (Konflikt zuerst, Verschiebung bleibt sichtbar).
  expect(
    schluessel(
      plan({ status: "verknuepft" }),
      link({
        sync_state: "konflikt",
        block_konflikt: true,
        verschiebung_offen: true,
      }),
    ),
  ).toEqual(["verknuepft", "konflikt", "verschoben"]);
  expect(
    schluessel(
      plan({ status: "abgesagt" }),
      link({ sync_state: "abgesagt", abgesagt: true }),
    ),
  ).toEqual(["abgesagt"]);
  expect(
    schluessel(
      plan({ status: "verknuepft" }),
      link({ sync_state: "verwaist", abgesagt: true }),
    ),
  ).toEqual(["verknuepft", "abgesagt", "verwaist"]);
  expect(
    schluessel(
      plan({ status: "verknuepft" }),
      link({ fremd_beobachtet: true }),
    ),
  ).toEqual(["verknuepft", "fremd"]);
  // Nach der Durchführung ändert der Kalender nichts mehr.
  expect(
    schluessel(
      plan({ status: "durchgefuehrt" }),
      link({ sync_state: "abgesagt", abgesagt: true }),
    ),
  ).toEqual(["durchgefuehrt"]);
  // Ältere Antworten ohne die Merker (optionale Felder fehlen).
  const alt = link();
  delete (alt as Partial<PlanLink>).verschiebung_offen;
  delete (alt as Partial<PlanLink>).block_konflikt;
  expect(schluessel(plan({ status: "verknuepft" }), alt)).toEqual([
    "verknuepft",
  ]);
});

test("Liste: Gruppen und Reihenfolge", () => {
  expect(listenGruppe(plan(), null)).toBe("aktiv");
  expect(listenGruppe(plan({ status: "abgesagt" }), null)).toBe("archiv");
  expect(listenGruppe(plan({ status: "versendet" }), null)).toBe("archiv");
  expect(
    listenGruppe(plan({ status: "verknuepft" }), link({ abgesagt: true })),
  ).toBe("archiv");

  const a = plan({ id: "a", updated_at: 5 });
  const b = plan({ id: "b", updated_at: 9 });
  const c = plan({ id: "c", updated_at: 1 });
  const d = plan({ id: "d", updated_at: 1 });
  const links = new Map<string, PlanLink | null>([
    ["a", link({ starts_at: 2000 })],
    ["b", null],
    ["c", link({ starts_at: 1000 })],
    ["d", null],
  ]);
  // Mit Termin nach Beginn, danach ohne Termin, zuletzt geänderte zuerst, Id als letzter Schlüssel.
  expect(sortiere([a, b, c, d], links).map((p) => p.id)).toEqual([
    "c",
    "a",
    "b",
    "d",
  ]);
});

test("Fehlercodes: bekannte bekommen ihren Satz, alles andere (auch Prototypnamen) den allgemeinen", () => {
  expect(fehlerSchluessel("titel_leer")).toBe("planung.fehler.titel_leer");
  expect(fehlerSchluessel("status_geaendert: x")).toBe(
    "planung.fehler.status_geaendert",
  );
  for (const roh of [
    "",
    "sql: no such table",
    "constructor",
    "toString: x",
    "__proto__",
  ])
    expect(fehlerSchluessel(roh), roh).toBe("planung.fehler.allgemein");
  for (const lang of ["de", "en"]) {
    for (const code of BEKANNTE_FEHLER)
      expect(
        blatt(lang, "planung", "fehler", code),
        `${lang}: ${code}`,
      ).toBeTruthy();
    expect(blatt(lang, "planung", "fehler", "allgemein")).toContain("{{code}}");
  }
});

test("Regelprüfung: jeder Code des Backends hat Sätze in Deutsch und Englisch", () => {
  const rust = fs.readFileSync(
    path.resolve(wurzel, "crates/besprechungsplanung/src/rules.rs"),
    "utf-8",
  );
  const backend = new Set(
    [...rust.matchAll(/add\(\s*"([a-z_]+)"/g)].map((m) => m[1]),
  );
  expect(backend.size).toBeGreaterThanOrEqual(15);
  expect([...backend].sort()).toEqual([...REGEL_CODES].sort());
  for (const lang of ["de", "en"]) {
    for (const code of backend)
      expect(
        blatt(lang, "planung", "regeln", code),
        `${lang}: ${code}`,
      ).toBeTruthy();
  }
  expect(regelSchluessel({ code: "agenda_zu_lang" })).toBe(
    "planung.regeln.agenda_zu_lang",
  );
  expect(regelSchluessel({ code: "constructor" })).toBeNull();
  const f = leeresFormular();
  f.duration_min = "25";
  f.agenda = [{ topic: "A", minutes: "30", owner: "" }];
  f.beginn = "05:30";
  const p = regelParameter({ code: "agenda_zu_lang" }, f);
  expect(p).toMatchObject({
    summe: 30,
    dauer: 25,
    von: "06",
    bis: "20",
    zeit: "05:30",
  });
  expect(uebergabeErlaubt([{ stufe: "warnung" }, { stufe: "info" }])).toBe(
    true,
  );
  expect(uebergabeErlaubt([{ stufe: "fehler" }])).toBe(false);
});

test("Einwilligungen: Ablehnung zuerst, dann offen, dann Zustimmung", () => {
  const c = (person: string, state: Consent["state"]): Consent => ({
    plan_id: "01HPLAN",
    person,
    state,
    channel: state === "zugestimmt" ? "einladung" : null,
    recorded_at: state === "zugestimmt" ? 1 : null,
  });
  expect(
    einwilligungenSortiert([
      c("b@example.org", "zugestimmt"),
      c("c@example.org", "offen"),
      c("a@example.org", "abgelehnt"),
      c("a2@example.org", "zugestimmt"),
    ]).map((x) => x.person),
  ).toEqual([
    "a@example.org",
    "c@example.org",
    "a2@example.org",
    "b@example.org",
  ]);
});

test("Texte: alle Schlüssel der Seite in Deutsch und Englisch, gleiche Struktur", () => {
  const form = (lang: string) => {
    const keys: string[] = [];
    const walk = (node: unknown, pre: string) => {
      if (node && typeof node === "object")
        for (const [k, v] of Object.entries(node)) walk(v, `${pre}.${k}`);
      else keys.push(pre);
    };
    walk(blatt(lang, "planung"), "planung");
    return keys.sort();
  };
  expect(form("de").length).toBeGreaterThan(150);
  expect(form("en")).toEqual(form("de"));
  expect(blatt("de", "sidebar", "planung")).toBe("Planung");
  expect(blatt("en", "sidebar", "planung")).toBe("Planning");
  const status = [
    "entwurf",
    "uebergeben",
    "verknuepft",
    "verschoben",
    "abgesagt",
    "durchgefuehrt",
    "protokolliert",
    "versendet",
    "konflikt",
    "verwaist",
    "fremd",
  ];
  for (const lang of ["de", "en"])
    for (const s of status) {
      expect(
        blatt(lang, "planung", "status", s),
        `${lang} status ${s}`,
      ).toBeTruthy();
      expect(
        blatt(lang, "planung", "statusHinweis", s),
        `${lang} hinweis ${s}`,
      ).toBeTruthy();
    }
});

test("Deutsche Texte: echte Umlaute, keine Umschreibung, kein Organisationsname", () => {
  const alle: string[] = [];
  const walk = (node: unknown) => {
    if (typeof node === "string") alle.push(node);
    else if (node && typeof node === "object")
      Object.values(node).forEach(walk);
  };
  walk(blatt("de", "planung"));
  const text = alle.join("\n");
  for (const wort of [
    "Prüfung",
    "Übergeben",
    "Löschen",
    "größer",
    "Klären",
    "bestätigen",
    "Zustimmung",
  ])
    expect(text, wort).toContain(wort);
  for (const falsch of [
    "Pruefung",
    "Uebergeben",
    "uebergeben ",
    "Loeschen",
    "groesser",
    "Klaeren",
    "Bestaetigen",
    "fuer ",
    "ueber ",
    "koennen",
    "waehle",
    "moeglich",
  ])
    expect(text, falsch).not.toContain(falsch);
  // Sichtbare Namen ohne Organisationsbezug (Konzept: organisationsneutral).
  for (const lang of ["de", "en"]) {
    const roh = JSON.stringify(blatt(lang, "planung"));
    expect(roh).not.toMatch(/\bwai\b/i);
  }
});
