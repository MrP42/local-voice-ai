import type { Page } from "@playwright/test";

// Tauri-Attrappe der Seite „Planung“ (lva#534 P2): die Befehle besprechungsplanung_* mit
// synthetischen Daten (example.org). Die Regelprüfung der Attrappe ist ein verkleinerter Nachbau
// der des Crates (rules.rs); die Seite zeigt nur, was die Attrappe zurückgibt.

export const T0 = 1_793_000_000_000;
const MIN = 60_000;

export interface MockAttendee {
    email: string | null;
    name: string | null;
    organizer: boolean;
    is_self: boolean;
    partstat: string | null;
}

interface MockCalEvent {
    key: string;
    source_id: string;
    uid: string;
    title: string;
    starts_at: number;
    ends_at: number;
    all_day: boolean;
    cancelled: boolean;
    location: string | null;
    join_url: string | null;
    description: string | null;
    attendees: MockAttendee[];
}

export interface MockPlan {
  id: string;
  meeting_id: string | null;
  title: string;
  goal_type: string;
  goal_text: string;
  expected_result: string | null;
  duration_min: number;
  classification: string;
  agenda: { minutes: number; topic: string; owner: string | null }[];
  roles: Record<string, string | null>;
  materials: { label: string; url: string | null; due_date: string | null }[];
  hints: Record<string, unknown>;
  transcription_planned: boolean;
  minutes_template_id: string | null;
  status: string;
  created_at: number;
  updated_at: number;
}

export function plan(
  id: string,
  title: string,
  extra: Partial<MockPlan> = {},
): MockPlan {
  return {
    id,
    meeting_id: null,
    title,
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
    materials: [],
    hints: {
      weiterleiten_erlaubt: null,
      barrierefreiheit: null,
      ergebnisfrist: null,
      ablageort: null,
    },
    transcription_planned: false,
    minutes_template_id: null,
    status: "entwurf",
    created_at: T0 - 86_400_000,
    updated_at: T0 - 3_600_000,
    ...extra,
  };
}

export function link(id: string, extra: Record<string, unknown> = {}) {
  return {
    plan_id: id,
    adapter: "ics_marker",
    source_id: "src1",
    remote_id: null,
    ical_uid: `${id}@example.org`,
    change_key: null,
    starts_at: T0 + 86_400_000,
    ends_at: T0 + 86_400_000 + 25 * MIN,
    location: "Raum 1.02",
    join_url: null,
    body_hash: null,
    last_seen_at: T0,
    last_change_at: T0,
    sync_state: "ok",
    verschiebung_offen: false,
    block_konflikt: false,
    abgesagt: false,
    instanz: null,
    fremd_beobachtet: false,
    ...extra,
  };
}

export function demoDaten() {
  const plaene = [
    plan("p_entwurf", "Abstimmung Pilotstart", {
      minutes_template_id: "vorlage-1",
    }),
    plan("p_uebergeben", "Jour fixe Betrieb", { status: "uebergeben" }),
    plan("p_verknuepft", "Planung Quartal 1", {
      status: "verknuepft",
      transcription_planned: true,
      duration_min: 50,
    }),
    plan("p_verschoben", "Review Datenschutz", { status: "verschoben" }),
    plan("p_konflikt", "Workshop Rollen", { status: "verknuepft" }),
    plan("p_abgesagt", "Kickoff Archiv", { status: "abgesagt" }),
    plan("p_fertig", "Retrospektive Oktober", {
      status: "durchgefuehrt",
      transcription_planned: true,
    }),
  ];
  const links: Record<string, unknown> = {
    p_uebergeben: null,
    p_verknuepft: link("p_verknuepft", {
      starts_at: T0 + 2 * 86_400_000,
      ends_at: T0 + 2 * 86_400_000 + 50 * MIN,
    }),
    p_verschoben: link("p_verschoben", {
      sync_state: "verschoben",
      verschiebung_offen: true,
      starts_at: T0 + 3 * 86_400_000,
      ends_at: T0 + 3 * 86_400_000 + 25 * MIN,
    }),
    p_konflikt: link("p_konflikt", {
      sync_state: "konflikt",
      block_konflikt: true,
      starts_at: T0 + 4 * 86_400_000,
      ends_at: T0 + 4 * 86_400_000 + 25 * MIN,
    }),
    p_abgesagt: link("p_abgesagt", { sync_state: "abgesagt", abgesagt: true }),
    p_fertig: link("p_fertig", {
      starts_at: T0 - 7 * 86_400_000,
      ends_at: T0 - 7 * 86_400_000 + 25 * MIN,
    }),
  };
  const consents: Record<string, unknown[]> = {
    p_verknuepft: [
      {
        plan_id: "p_verknuepft",
        person: "a@example.org",
        state: "zugestimmt",
        channel: "einladung",
        recorded_at: T0 - 3_600_000,
      },
      {
        plan_id: "p_verknuepft",
        person: "b@example.org",
        state: "offen",
        channel: null,
        recorded_at: null,
      },
      {
        plan_id: "p_verknuepft",
        person: "c@example.org",
        state: "abgelehnt",
        channel: "mail",
        recorded_at: T0 - 7_200_000,
      },
    ],
  };
  const termine: MockCalEvent[] = [{
    key: "src1:p_verknuepft",
    source_id: "src1",
    uid: "p_verknuepft@example.org",
    title: "Planung Quartal 1",
    starts_at: T0 + 2 * 86_400_000,
    ends_at: T0 + 2 * 86_400_000 + 50 * MIN,
    all_day: false,
    cancelled: false,
    location: "Raum 1.02",
    join_url: null,
    description: null,
    attendees: ["a@example.org", "b@example.org", "c@example.org"].map((email) => ({
      email, name: null, organizer: false, is_self: false, partstat: null,
    })),
  }];
  const verlauf: Record<string, unknown[]> = {
    p_verknuepft: [consents.p_verknuepft[2], consents.p_verknuepft[0]].map((c) => {
      const einwilligung = c as { recorded_at: number };
      return { einwilligung: { ...einwilligung }, festgehalten_am: einwilligung.recorded_at };
    }),
  };
  const ergaenzt: Record<string, string[]> = {};
  const protokoll: Record<string, MockProtokoll> = {};
  return { plaene, links, consents, termine, verlauf, ergaenzt, protokoll };
}

/** Stand des Protokolls einer Planung in der Attrappe (P5, lva#736). */
export interface MockProtokoll {
  vorlage_id: string | null;
  erreicht: string | null;
  /** Das Protokoll (Markdown); `null`, solange nichts erzeugt wurde. */
  text: string | null;
  version: number;
  /** Zählt jede Änderung des Textes; die Freigabe merkt sich den Zählerstand. */
  aenderung: number;
  freigabe: number | null;
}

/**
 * Planungen für die Protokoll-Seite: `p_prot` (Besprechung vorhanden, alle haben zugestimmt) und
 * `p_prot_offen` (Besprechung vorhanden, eine Zustimmung fehlt: das Tor sperrt das Modell),
 * `p_prot_ohne` (noch keine Besprechung).
 */
function protokollDaten(d: ReturnType<typeof demoDaten>) {
  const zusage = (id: string, person: string) => ({
    plan_id: id,
    person,
    state: "zugestimmt",
    channel: "einladung",
    recorded_at: T0 - 3_600_000,
  });
  const termin = (key: string, titel: string, adressen: (string | null)[]) => ({
    key,
    source_id: "src1",
    uid: `${key}@example.org`,
    title: titel,
    starts_at: T0 - 86_400_000,
    ends_at: T0 - 86_400_000 + 25 * MIN,
    all_day: false,
    cancelled: false,
    location: null,
    join_url: null,
    description: null,
    attendees: adressen.map((email) => ({
      email,
      name: null,
      organizer: false,
      is_self: false,
      partstat: null,
    })),
  });
  const leer = (): MockProtokoll => ({
    vorlage_id: null,
    erreicht: null,
    text: null,
    version: 0,
    aenderung: 0,
    freigabe: null,
  });
  d.plaene.push(
    plan("p_prot", "Abstimmung Posteingang", {
      status: "durchgefuehrt",
      meeting_id: "m_prot",
      transcription_planned: true,
    }),
    plan("p_prot_offen", "Jour fixe Archiv", {
      status: "durchgefuehrt",
      meeting_id: "m_offen",
      transcription_planned: true,
    }),
    plan("p_prot_ohne", "Workshop Ablage", { status: "verknuepft" }),
  );
  for (const id of ["p_prot", "p_prot_offen", "p_prot_ohne"]) {
    (d.links as Record<string, unknown>)[id] = link(id, {
      starts_at: T0 - 86_400_000,
      ends_at: T0 - 86_400_000 + 25 * MIN,
    });
    d.protokoll[id] = leer();
  }
  d.consents.p_prot = [
    zusage("p_prot", "a@example.org"),
    zusage("p_prot", "b@example.org"),
  ];
  d.consents.p_prot_offen = [zusage("p_prot_offen", "a@example.org")];
  d.termine.push(
    // Dublette in anderer Schreibweise, eine Person ohne Adresse
    termin("src1:p_prot", "Abstimmung Posteingang", [
      "a@example.org",
      "A@Example.org",
      "b@example.org",
      null,
    ]),
    termin("src1:p_prot_offen", "Jour fixe Archiv", [
      "a@example.org",
      "b@example.org",
    ]),
  );
}

export async function installMock(
  page: Page,
  opts: {
    theme?: "light" | "dark";
    leer?: boolean;
    notizen?: boolean;
    protokoll?: boolean;
  } = {},
) {
  const daten: ReturnType<typeof demoDaten> = opts.leer
    ? {
        plaene: [],
        links: {},
        consents: {},
        termine: [],
        verlauf: {},
        ergaenzt: {},
        protokoll: {},
      }
    : demoDaten();
  if (opts.protokoll && !opts.leer) protokollDaten(daten);
  if (opts.notizen && !opts.leer) {
    daten.plaene.push(plan("p_notizen", "Jour fixe Notizen", { status: "verknuepft" }));
    (daten.links as Record<string, unknown>).p_notizen = link("p_notizen");
    daten.termine.push({
      key: "src1:p_notizen",
      source_id: "src1",
      uid: "p_notizen@example.org",
      title: "Jour fixe Notizen",
      starts_at: T0 + 86_400_000,
      ends_at: T0 + 86_400_000 + 25 * MIN,
      all_day: false,
      cancelled: false,
      location: null,
      join_url: null,
      description: null,
      attendees: [{
        email: "a@example.org", name: null, organizer: false, is_self: false, partstat: null,
      }],
    });
  }
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.addInitScript(
    ({ daten, theme }) => {
      // Die Vorschau liegt in einem Rahmen ohne Rechte (`sandbox=""`): dort gibt es weder
      // localStorage noch die Attrappe, und der Initialisierungsskript läuft auch dort.
      if (window.top !== window) return;
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
      w.__daten = daten;
      // Tests halten einzelne Antworten an oder lassen sie ohne Inhaltsdaten fehlschlagen.
      w.__steuerung = {};
      w.__freigaben = {};
      window.localStorage.setItem("lva.ui.section", "planung");

      const pruefen = (input: any, beginn: any) => {
        const h: { code: string; stufe: string; text: string }[] = [];
        const add = (code: string, stufe: string) =>
          h.push({ code, stufe, text: code });
        if (!String(input.title).trim()) add("titel_leer", "fehler");
        if (!String(input.goal_text).trim()) add("ziel_fehlt", "warnung");
        if (!String(input.expected_result ?? "").trim())
          add("ergebnis_fehlt", "info");
        if (input.duration_min === 0) add("dauer_null", "fehler");
        else if (![25, 50].includes(input.duration_min))
          add("dauer_unueblich", "warnung");
        if (input.agenda.length === 0) add("agenda_leer", "warnung");
        else {
          if (input.agenda.some((a: any) => !String(a.topic).trim()))
            add("agenda_punkt_leer", "fehler");
          const summe = input.agenda.reduce(
            (s: number, a: any) => s + a.minutes,
            0,
          );
          if (summe > input.duration_min && input.duration_min > 0)
            add("agenda_zu_lang", "warnung");
        }
        if (input.transcription_planned)
          add("transkription_einwilligung", "info");
        if (beginn && beginn.minute !== 0 && beginn.minute !== 30)
          add("beginn_krumm", "info");
        return h;
      };
      const vorschau = (input: any, id: string | null) => {
        const marke = `lva-plan:${id ?? "vorschau"}`;
        const text = [
          "— Besprechungsplanung —",
          input.title,
          `Ziel: ${input.goal_text}`,
          `Dauer: ${input.duration_min} Minuten`,
          "",
          marke,
        ].join("\n");
        return {
          html: `<div style="font-family:'Segoe UI',sans-serif"><p><b>${input.title}</b></p><p>Ziel: ${input.goal_text}</p><p>${marke}</p></div>`,
          text,
          block_hash: "h",
        };
      };

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
            const steuerung = w.__steuerung[cmd];
            if (steuerung?.pause) {
              await new Promise<void>((resolve) => {
                (w.__freigaben[cmd] ??= []).push(resolve);
              });
            }
            if (steuerung?.fehlerText) throw new Error(steuerung.fehlerText);
            if (steuerung?.fehler) throw new Error("mock_abruf_fehlgeschlagen");
            const d = w.__daten as {
              plaene: any[];
              links: Record<string, any>;
              consents: Record<string, any[]>;
              termine: MockCalEvent[];
              verlauf: Record<string, any[]>;
              ergaenzt: Record<string, string[]>;
              protokoll: Record<string, MockProtokoll>;
            };
            // --- P5 (lva#736): Protokoll aus der Agenda, "Ziel erreicht", Freigabe, Entwurf ---
            const planDerBesprechung = (meetingId: string) =>
              d.plaene.find((p) => p.meeting_id === meetingId);
            const kopf = (p: any, erreicht: string | null) =>
              "# Protokoll: " +
              p.title +
              "\n\n" +
              "**Ziel (Entscheiden):** " +
              p.goal_text +
              "\n" +
              "**Ziel erreicht:** " +
              (erreicht ?? "noch nicht bewertet") +
              "\n" +
              "**Klassifizierung:** " +
              p.classification +
              "\n";
            const protokollStand = (id: string) => {
              const p = d.plaene.find((x) => x.id === id);
              if (!p) throw new Error("plan_nicht_gefunden");
              const z = (d.protokoll[id] ??= {
                vorlage_id: null,
                erreicht: null,
                text: null,
                version: 0,
                aenderung: 0,
                freigabe: null,
              });
              const freigegeben = z.text !== null && z.freigabe === z.aenderung;
              return {
                plan_id: id,
                meeting_id: p.meeting_id,
                vorlage_id: z.vorlage_id,
                dokument_id: z.text === null ? null : `doc_${id}_${z.version}`,
                dokument_version: z.text === null ? null : z.version,
                erreicht: z.erreicht,
                kopf_fehlt: false,
                freigegeben,
                freigegeben_am: freigegeben ? 1 : null,
                geaendert_seit_freigabe: z.freigabe !== null && !freigegeben,
              };
            };
            if (cmd === "besprechungsplanung_protokoll_stand")
              return protokollStand(args?.id);
            if (cmd === "besprechungsplanung_protokollvorlage") {
              const p = d.plaene.find((x) => x.id === args?.id);
              if (!p) throw new Error("plan_nicht_gefunden");
              if (!p.meeting_id) throw new Error("besprechung_fehlt");
              const z =
                d.protokoll[p.id] ?? (protokollStand(p.id), d.protokoll[p.id]);
              z.vorlage_id = `vorlage_${p.id}`;
              return {
                template_id: z.vorlage_id,
                meeting_id: p.meeting_id,
                abschnitte: p.agenda.length + 3,
              };
            }
            if (cmd === "meetings_generate_minutes") {
              const p = planDerBesprechung(args?.meetingId);
              if (!p) throw new Error("meeting_not_found");
              // Einwilligungs-Tor aus P3: ohne Zustimmung aller Termin-Teilnehmenden kein Modell.
              const termin =
                d.termine.find(
                  (ev) => ev.uid === `${(d.links[p.id] ?? {}).ical_uid}`,
                ) ?? d.termine.find((ev) => ev.key === `src1:${p.id}`);
              const personen = new Set<string>();
              for (const a of termin?.attendees ?? [])
                if (a.email) personen.add(a.email.toLowerCase());
              const zugestimmt = (d.consents[p.id] ?? [])
                .filter((c) => c.state === "zugestimmt")
                .map((c) => c.person);
              if ([...personen].some((x) => !zugestimmt.includes(x)))
                throw new Error("nur_notizen: offen");
              const z = d.protokoll[p.id];
              z.version += 1;
              z.aenderung += 1;
              z.text = kopf(p, z.erreicht) + "\n## 1. Stand\n\nText.\n";
              return { id: `doc_${p.id}_${z.version}`, version: z.version };
            }
            if (cmd === "besprechungsplanung_erreicht_setzen") {
              const stand = protokollStand(args?.id);
              const z = d.protokoll[args?.id];
              z.erreicht = args?.erreicht ?? null;
              if (z.text !== null) z.aenderung += 1;
              return { ...stand, ...protokollStand(args?.id) };
            }
            if (cmd === "besprechungsplanung_protokoll_freigeben") {
              const stand = protokollStand(args?.id);
              if (!stand.meeting_id) throw new Error("besprechung_fehlt");
              const z = d.protokoll[args?.id];
              if (z.text === null) throw new Error("protokoll_fehlt");
              z.freigabe = z.aenderung;
              d.plaene.find((x) => x.id === args?.id).status = "protokolliert";
              return protokollStand(args?.id);
            }
            if (cmd === "besprechungsplanung_protokoll_entwurf") {
              const stand = protokollStand(args?.id);
              if (!stand.freigegeben)
                throw new Error("protokoll_nicht_freigegeben");
              const p = d.plaene.find((x) => x.id === args?.id);
              const termin = d.termine.find((ev) => ev.key === `src1:${p.id}`);
              const adressen: string[] = [];
              let ohne = 0;
              for (const a of termin?.attendees ?? []) {
                const adresse = (a.email ?? "").trim().toLowerCase();
                if (!adresse) ohne += 1;
                else if (!adressen.includes(adresse)) adressen.push(adresse);
              }
              return {
                entwurf: {
                  to: adressen,
                  subject: `Protokoll: ${p.title} (2026-11-12)`,
                  body_text: `Klassifizierung: ${p.classification}\n\n${d.protokoll[p.id].text}`,
                  body_html: "",
                },
                ohne_adresse: ohne,
                termin_gefunden: !!termin,
              };
            }
            // `true` = der Text war zu lang für den Mail-Link und liegt in der Zwischenablage
            // (`__steuerung.meeting_followup_open = { gekuerzt: true }`).
            if (cmd === "meeting_followup_open")
              return steuerung?.gekuerzt === true;
            if (cmd === "besprechungsplanung_anwesende_ergaenzen") {
                const liste = (d.ergaenzt[args?.meetingId] ??= []);
                for (const eingabe of args?.personen ?? []) {
                    const person = eingabe.startsWith("id:") ? eingabe : eingabe.toLowerCase();
                    if (!liste.includes(person)) liste.push(person);
                }
                return null;
            }
            // Das Starttor prüft den aktuellen Nachweis, unabhängig von der Oberfläche.
            const identitaet = (eingabe: string | null) => {
              const wert = (eingabe ?? "").trim().toLowerCase();
              if (/^id:[a-z0-9_-]{1,64}$/.test(wert)) return wert;
              return /^[^\s@]+@[^\s@.][^\s@]*\.[^\s@.]+$/.test(wert) ? wert : null;
            };
            const startModus = (id: string, anwesende: (string | null)[]) => {
              const offen: string[] = [];
              const abgelehnt: string[] = [];
              if (!d.plaene.find((p) => p.id === id)?.transcription_planned) {
                return { modus: "nur_notizen", nicht_geplant: true, offen, abgelehnt };
              }
              const gesehen = new Set<string>();
              for (const a of anwesende) {
                const person = identitaet(a);
                if (person === null) {
                  offen.push("unbekannt");
                  continue;
                }
                if (gesehen.has(person)) continue;
                gesehen.add(person);
                const stand = (d.consents[id] ?? []).find((c) => c.person === person)?.state;
                if (stand === "abgelehnt") abgelehnt.push(person);
                else if (stand !== "zugestimmt") offen.push(person);
              }
              return anwesende.length === 0 || offen.length > 0 || abgelehnt.length > 0
                ? { modus: "nur_notizen", nicht_geplant: false, offen, abgelehnt }
                : { modus: "mit_transkription" };
            };
            if (cmd === "besprechungsplanung_einwilligungs_verlauf")
              return JSON.parse(JSON.stringify(d.verlauf[args?.id] ?? []));
            if (cmd === "besprechungsplanung_start_modus")
              return startModus(args?.id, args?.anwesende ?? []);
            if (cmd === "calendar_upcoming") return d.termine ?? [];
            if (cmd === "besprechungsplanung_notizen_starten") {
              d.plaene.find((p) => p.id === args?.id).meeting_id = "m_notizen";
              return { id: "m_notizen", title: args?.title };
            }
            if (cmd === "meetings_start_from_event") {
              if (!args?.consentConfirmed) throw new Error("consent_required");
              const termin = d.termine.find((ev) => ev.key === args?.eventKey);
              const id = Object.keys(d.links).find(
                (id) => termin && d.links[id]?.ical_uid === termin.uid,
              );
              const modus = startModus(id ?? "", (termin?.attendees ?? []).map((a) => a.email));
              if (modus.modus === "nur_notizen") {
                const grund = modus.nicht_geplant ? "nicht_geplant"
                  : modus.abgelehnt?.length ? "abgelehnt" : "offen";
                throw new Error(`nur_notizen: ${grund}`);
              }
              return { id: "m_aufnahme", title: args?.title ?? termin?.title ?? "Besprechung" };
            }
            if (cmd === "get_app_settings" || cmd === "get_default_settings")
              return JSON.parse(JSON.stringify(settings));
            if (cmd === "plugin:os|locale") return "de-DE";
            if (cmd === "plugin:app|version") return "0.23.0";
            if (cmd.includes("permission")) return true;
            if (cmd === "plugin:event|listen") return Number(args?.handler);
            if (cmd === "plugin:event|unlisten") return null;
            if (cmd === "besprechungsplanung_liste")
              return JSON.parse(JSON.stringify(d.plaene));
            if (cmd === "besprechungsplanung_verknuepfung")
              return JSON.parse(JSON.stringify(d.links[args?.id] ?? null));
            if (cmd === "besprechungsplanung_einwilligungen")
              return JSON.parse(JSON.stringify(d.consents[args?.id] ?? []));
            if (cmd === "besprechungsplanung_einwilligung_setzen") {
              const c = args?.einwilligung;
              const liste = (d.consents[c.plan_id] ??= []);
              const index = liste.findIndex((alt) => alt.person === c.person);
              if (index < 0) liste.push({ ...c });
              else liste[index] = { ...c };
              (d.verlauf[c.plan_id] ??= []).push({
                einwilligung: { ...c }, festgehalten_am: Date.now(),
              });
              return null;
            }
            if (cmd === "besprechungsplanung_pruefen")
              return pruefen(args?.input, args?.beginn);
            if (cmd === "besprechungsplanung_vorschau")
              return vorschau(args?.input, args?.id);
            if (cmd === "besprechungsplanung_anlegen") {
              const p = {
                ...args?.input,
                id: `neu${d.plaene.length}`,
                meeting_id: null,
                status: "entwurf",
                created_at: Date.now(),
                updated_at: Date.now(),
              };
              d.plaene.push(p);
              return p;
            }
            if (cmd === "besprechungsplanung_aendern") {
              const i = d.plaene.findIndex((p) => p.id === args?.id);
              d.plaene[i] = {
                ...d.plaene[i],
                ...args?.input,
                updated_at: Date.now(),
              };
              return d.plaene[i];
            }
            if (cmd === "besprechungsplanung_einladung") {
              const p = d.plaene.find((x) => x.id === args?.id);
              p.status = "uebergeben";
              return vorschau(p, p.id);
            }
            if (cmd === "besprechungsplanung_loeschen") {
              d.plaene = d.plaene.filter((p) => p.id !== args?.id);
              return null;
            }
            if (cmd === "besprechungsplanung_verschiebung_bestaetigen") {
              const l = d.links[args?.id];
              l.verschiebung_offen = false;
              l.sync_state = "ok";
              d.plaene.find((p) => p.id === args?.id).status = "verknuepft";
              return l;
            }
            if (cmd === "besprechungsplanung_abgleichen")
              return {
                geprueft: 4,
                geaendert: 1,
                ereignisse: [],
                mehrdeutig: [],
                uebersprungen: 0,
                fehler: [],
              };
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
      daten,
      theme: opts.theme ?? "light",
    },
  );
}

export async function setzeTermin(page: Page, key: string, attendees: MockAttendee[]) {
    await page.evaluate(({ key, attendees }) => {
        const termin = (window as any).__daten.termine.find((ev: MockCalEvent) => ev.key === key);
        if (!termin) throw new Error("mock_termin_fehlt");
        termin.attendees = attendees;
    }, { key, attendees });
}

export async function ergaenztVon(page: Page, meetingId: string): Promise<string[]> {
    return page.evaluate((id) => (window as any).__daten.ergaenzt[id] ?? [], meetingId);
}
