import type {
  Consent,
  GoalType,
  Hinweis,
  LokaleZeit,
  Plan,
  PlanInput,
  PlanLink,
  PlanStatus,
} from "@/bindings";

/**
 * Reine Logik der Planungsseite (lva#534 P2): Formularzustand <-> Eingabe des Backends, Status-
 * Marken, Sortierung, Fehlercodes. Ohne React und ohne Tauri, damit sie ohne Browser prüfbar ist.
 */

/** Zeitfeld der Agenda und Dauer als Text: das Feld darf leer oder halb getippt sein. */
export interface AgendaZeile {
  topic: string;
  minutes: string;
  owner: string;
}

export interface UnterlageZeile {
  label: string;
  url: string;
  due_date: string;
}

/** `""` = nicht angegeben. */
export type Weiterleiten = "" | "ja" | "nein";

export interface PlanForm {
  title: string;
  goal_type: GoalType;
  goal_text: string;
  expected_result: string;
  duration_min: string;
  classification: string;
  /** Voraussichtlicher Beginn `HH:MM` (nur für die Regelprüfung, kein Teil der Planung). */
  beginn: string;
  agenda: AgendaZeile[];
  moderation: string;
  ergebnissicherung: string;
  entscheidung: string;
  materials: UnterlageZeile[];
  weiterleiten: Weiterleiten;
  barrierefreiheit: string;
  ergebnisfrist: string;
  ablageort: string;
  transcription_planned: boolean;
  minutes_template_id: string | null;
}

export const ZIEL_ARTEN: readonly GoalType[] = [
  "klaeren",
  "feedback",
  "entscheiden",
  "information",
];

/** Empfohlene Dauern und Arbeitszeitrahmen: die Vorgaben von `Regeln::default()` im Crate. */
export const EMPFOHLENE_DAUERN: readonly number[] = [25, 50];
export const MAX_DAUER_MIN = 8 * 60;
export const RAHMEN_VON_STUNDE = 6;
export const RAHMEN_BIS_STUNDE = 20;

/** Wertebereich der Klassifizierung (Vorgabe `intern`); frei erweiterbar durch gespeicherte Werte. */
export const KLASSIFIZIERUNGEN: readonly string[] = [
  "öffentlich",
  "intern",
  "vertraulich",
  "streng vertraulich",
];

export function leeresFormular(): PlanForm {
  return {
    title: "",
    goal_type: "klaeren",
    goal_text: "",
    expected_result: "",
    duration_min: "25",
    classification: "intern",
    beginn: "",
    agenda: [],
    moderation: "",
    ergebnissicherung: "",
    entscheidung: "",
    materials: [],
    weiterleiten: "",
    barrierefreiheit: "",
    ergebnisfrist: "",
    ablageort: "",
    transcription_planned: false,
    minutes_template_id: null,
  };
}

const text = (v: string | null | undefined): string => v ?? "";

export function formularAusPlan(plan: Plan, beginn = ""): PlanForm {
  return {
    title: plan.title,
    goal_type: plan.goal_type,
    goal_text: plan.goal_text,
    expected_result: text(plan.expected_result),
    duration_min: String(plan.duration_min),
    classification: plan.classification,
    beginn,
    agenda: plan.agenda.map((a) => ({
      topic: a.topic,
      minutes: String(a.minutes),
      owner: text(a.owner),
    })),
    moderation: text(plan.roles.moderation),
    ergebnissicherung: text(plan.roles.ergebnissicherung),
    entscheidung: text(plan.roles.entscheidung),
    materials: plan.materials.map((m) => ({
      label: m.label,
      url: text(m.url),
      due_date: datumTextAusIso(text(m.due_date)),
    })),
    weiterleiten:
      plan.hints.weiterleiten_erlaubt == null
        ? ""
        : plan.hints.weiterleiten_erlaubt
          ? "ja"
          : "nein",
    barrierefreiheit: text(plan.hints.barrierefreiheit),
    ergebnisfrist: datumTextAusIso(text(plan.hints.ergebnisfrist)),
    ablageort: text(plan.hints.ablageort),
    transcription_planned: plan.transcription_planned,
    minutes_template_id: plan.minutes_template_id,
  };
}

/** Ganze Zahl ab 0; alles andere (leer, Buchstaben, Dezimalzahl, negativ) ist 0. */
export function minutenAusText(s: string): number {
  const t = s.trim();
  if (!/^\d{1,6}$/.test(t)) return 0;
  return Number(t);
}

const leerZuNull = (s: string): string | null => {
  const t = s.trim();
  return t === "" ? null : t;
};

/**
 * Das, was das Backend speichert und prüft. Zeilen ohne jeden Inhalt (eben angelegt, nicht
 * ausgefüllt) entfallen; eine Zeile mit Zeitbox aber ohne Thema bleibt und erzeugt den Regelfehler
 * `agenda_punkt_leer`, statt still zu verschwinden.
 */
export function eingabeAusFormular(f: PlanForm): PlanInput {
  const agenda = f.agenda
    .filter(
      (a) =>
        a.topic.trim() !== "" || a.owner.trim() !== "" || a.minutes.trim() !== "",
    )
    .map((a) => ({
      topic: a.topic.trim(),
      minutes: minutenAusText(a.minutes),
      owner: leerZuNull(a.owner),
    }));
  const materials = f.materials
    .filter((m) => m.label.trim() !== "" || m.url.trim() !== "")
    .map((m) => ({
      label: m.label.trim() !== "" ? m.label.trim() : m.url.trim(),
      url: leerZuNull(m.url),
      due_date: datumIsoAusText(m.due_date),
    }));
  return {
    title: f.title.trim(),
    goal_type: f.goal_type,
    goal_text: f.goal_text.trim(),
    expected_result: leerZuNull(f.expected_result),
    duration_min: minutenAusText(f.duration_min),
    classification: f.classification.trim() || "intern",
    agenda,
    roles: {
      moderation: leerZuNull(f.moderation),
      ergebnissicherung: leerZuNull(f.ergebnissicherung),
      entscheidung: leerZuNull(f.entscheidung),
    },
    materials,
    hints: {
      weiterleiten_erlaubt:
        f.weiterleiten === "" ? null : f.weiterleiten === "ja",
      barrierefreiheit: leerZuNull(f.barrierefreiheit),
      ergebnisfrist: datumIsoAusText(f.ergebnisfrist),
      ablageort: leerZuNull(f.ablageort),
    },
    transcription_planned: f.transcription_planned,
    minutes_template_id: f.minutes_template_id,
  };
}

/** Summe der Zeitboxen der ausgefüllten Agendapunkte. */
export function agendaSumme(f: PlanForm): number {
  return (eingabeAusFormular(f).agenda ?? []).reduce(
    (s, a) => s + a.minutes,
    0,
  );
}

/** `HH:MM` aus dem Zeitfeld; ohne gültige Uhrzeit `null` (die Zeitregeln entfallen dann). */
export function beginnAusText(s: string): LokaleZeit | null {
  const m = /^((?:[01]\d|2[0-3])):([0-5]\d)$/.exec(s);
  if (!m) return null;
  const stunde = Number(m[1]);
  const minute = Number(m[2]);
  if (stunde > 23 || minute > 59) return null;
  return { stunde, minute };
}

/** Ortszeit `HH:MM` eines UTC-Zeitpunkts (ms), für das Zeitfeld. */
export function beginnTextAusMs(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "";
  return new Intl.DateTimeFormat("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(ms));
}

/** Feste deutsche Datumseingabe in beiden Sprachen, unabhängig vom Browser. */
export function datumTextAusIso(s: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
  return m ? `${m[3]}.${m[2]}.${m[1]}` : s;
}

export function datumIsoAusText(s: string): string | null {
  const m = /^(\d{2})\.(\d{2})\.(\d{4})$/.exec(s);
  if (!m || Number(m[3]) === 0) return null;
  const iso = `${m[3]}-${m[2]}-${m[1]}`;
  const d = new Date(`${iso}T00:00:00Z`);
  return Number.isFinite(d.getTime()) && d.toISOString().slice(0, 10) === iso
    ? iso
    : null;
}

/** Leerer Beginn ist optional; ungültige, eingegebene Werte sperren das Speichern. */
export function formularGueltig(f: PlanForm): boolean {
  return (
    (f.beginn === "" || beginnAusText(f.beginn) !== null) &&
    (f.ergebnisfrist === "" || datumIsoAusText(f.ergebnisfrist) !== null) &&
    f.materials.every(
      (m) => m.due_date === "" || datumIsoAusText(m.due_date) !== null,
    )
  );
}

/** Alle Zeitangaben der Planung bleiben auch im englischen Browser im 24-Stunden-Format. */
export function planungsZeitText(ms: number): string {
  return new Intl.DateTimeFormat("de-DE", {
    dateStyle: "medium",
    timeStyle: "short",
    hourCycle: "h23",
  }).format(new Date(ms));
}

/** Wer die Planung nicht mehr ändern darf: sie ist durchgeführt (das Protokoll bezieht sich auf sie). */
export function istAbgeschlossen(status: PlanStatus): boolean {
  return (
    status === "durchgefuehrt" ||
    status === "protokolliert" ||
    status === "versendet"
  );
}

export type Ton = "neutral" | "info" | "success" | "warning" | "danger";

/** Eine Statusmarke der Anzeige; `schluessel` ist der Teil der i18n-Schlüssel (`planung.status.<..>`). */
export interface StatusMarke {
  schluessel: PlanStatus | "konflikt" | "verwaist" | "fremd";
  ton: Ton;
}

const STATUS_TON: Record<PlanStatus, Ton> = {
  entwurf: "neutral",
  uebergeben: "info",
  verknuepft: "success",
  verschoben: "warning",
  abgesagt: "danger",
  durchgefuehrt: "success",
  protokolliert: "success",
  versendet: "success",
};

/**
 * Die Marken zu Status und Verknüpfung: zuerst der Status der Planung, danach, was der Abgleich
 * zusätzlich meldet (Konflikt, unbestätigte Verschiebung, Absage, verwaister Termin, fremder
 * Termin), jeweils nur einmal. Nach der Durchführung ändert der Kalender nichts mehr
 * (Konzept 3.4), dann steht nur der Status.
 */
export function statusMarken(
  plan: Pick<Plan, "status">,
  link: PlanLink | null,
): StatusMarke[] {
  const marken: StatusMarke[] = [
    { schluessel: plan.status, ton: STATUS_TON[plan.status] },
  ];
  if (!link || istAbgeschlossen(plan.status)) return marken;
  const hat = (s: StatusMarke["schluessel"]) =>
    marken.some((m) => m.schluessel === s);
  if (link.block_konflikt || link.sync_state === "konflikt")
    marken.push({ schluessel: "konflikt", ton: "danger" });
  if (
    (link.verschiebung_offen || link.sync_state === "verschoben") &&
    !hat("verschoben")
  )
    marken.push({ schluessel: "verschoben", ton: "warning" });
  if ((link.abgesagt || link.sync_state === "abgesagt") && !hat("abgesagt"))
    marken.push({ schluessel: "abgesagt", ton: "danger" });
  if (link.sync_state === "verwaist")
    marken.push({ schluessel: "verwaist", ton: "warning" });
  if (link.fremd_beobachtet) marken.push({ schluessel: "fremd", ton: "info" });
  return marken;
}

/** Ist die Verschiebung noch nicht zur Kenntnis genommen? (Dann gibt es „Bestätigen“.) */
export function verschiebungOffen(link: PlanLink | null): boolean {
  return !!link && (link.verschiebung_offen ?? false);
}

export type ListenGruppe = "aktiv" | "archiv";

/** Archiv: durchgeführt, protokolliert, versendet oder abgesagt. Alles andere ist aktiv. */
export function listenGruppe(
  plan: Pick<Plan, "status">,
  link: PlanLink | null,
): ListenGruppe {
  if (istAbgeschlossen(plan.status) || plan.status === "abgesagt")
    return "archiv";
  if (link && !!(link.abgesagt ?? false)) return "archiv";
  return "aktiv";
}

/**
 * Reihenfolge der Liste: Planungen mit Termin nach Beginn (der nächste zuerst), danach die ohne
 * Termin, zuletzt geänderte zuerst. Deterministisch (Id als letzter Schlüssel).
 */
export function sortiere(
  plaene: Plan[],
  links: ReadonlyMap<string, PlanLink | null>,
): Plan[] {
  const beginn = (p: Plan) => links.get(p.id)?.starts_at ?? null;
  return [...plaene].sort((a, b) => {
    const ba = beginn(a);
    const bb = beginn(b);
    if (ba !== null && bb !== null && ba !== bb) return ba - bb;
    if (ba !== null && bb === null) return -1;
    if (ba === null && bb !== null) return 1;
    if (a.updated_at !== b.updated_at) return b.updated_at - a.updated_at;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });
}

/** Fehlercodes des Backends, für die die Seite einen eigenen Satz hat (`planung.fehler.<code>`). */
export const BEKANNTE_FEHLER: ReadonlySet<string> = new Set([
  "titel_leer",
  "titel_zu_lang",
  "dauer_null",
  "dauer_zu_lang",
  "agenda_punkt_leer",
  "plan_abgeschlossen",
  "plan_nicht_gefunden",
  "plan_id_ungueltig",
  "status_geaendert",
  "status_uebergang_ungueltig",
  "status_nur_durch_abgleich",
  "verknuepfung_fehlt",
  "daten_beschaedigt",
  "foreign_keys_nicht_aktiv",
]);

/** Der Code vor dem ersten Doppelpunkt (`<code>` oder `<code>: <detail>`). */
export function fehlerCode(roh: string): string {
  return roh.split(":")[0].trim();
}

/** i18n-Schlüssel eines Fehlers der Planungsbefehle; unbekannte Codes bekommen den allgemeinen. */
export function fehlerSchluessel(roh: string): string {
  const code = fehlerCode(roh);
  // Ein Set, keine Objekttabelle: `constructor` oder `toString` sind keine bekannten Codes.
  return BEKANNTE_FEHLER.has(code)
    ? `planung.fehler.${code}`
    : "planung.fehler.allgemein";
}

/** Codes der Regelprüfung (`rules.rs`), für die die Seite einen Satz je Sprache hat. */
export const REGEL_CODES: readonly string[] = [
  "titel_leer",
  "titel_zu_lang",
  "ziel_fehlt",
  "ergebnis_fehlt",
  "dauer_null",
  "dauer_zu_lang",
  "dauer_unueblich",
  "agenda_leer",
  "agenda_punkt_leer",
  "agenda_zeitbox_null",
  "agenda_zu_lang",
  "transkription_einwilligung",
  "beginn_ungueltig",
  "beginn_krumm",
  "ausserhalb_rahmen",
];

const REGEL_SET: ReadonlySet<string> = new Set(REGEL_CODES);

/** Satz zu einem Hinweis der Regelprüfung; ein unbekannter Code zeigt den Text des Backends. */
export function regelSchluessel(h: Pick<Hinweis, "code">): string | null {
  return REGEL_SET.has(h.code) ? `planung.regeln.${h.code}` : null;
}

/** Werte für die Platzhalter der Regelsätze (Zahlen aus dem Formular und den Vorgaben). */
export function regelParameter(
  h: Pick<Hinweis, "code">,
  f: PlanForm,
): Record<string, string | number> {
  const p2 = (n: number) => String(n).padStart(2, "0");
  const b = beginnAusText(f.beginn);
  return {
    max: MAX_DAUER_MIN,
    a: EMPFOHLENE_DAUERN[0],
    b: EMPFOHLENE_DAUERN[1],
    summe: agendaSumme(f),
    dauer: minutenAusText(f.duration_min),
    titelMax: 200,
    von: p2(RAHMEN_VON_STUNDE),
    bis: p2(RAHMEN_BIS_STUNDE),
    zeit: b ? `${p2(b.stunde)}:${p2(b.minute)}` : f.beginn.trim(),
  };
}

/** Anzahl je Stufe, schwerste zuerst (für die Kopfzeile der Regelhinweise). */
export function stufenZaehlung(hinweise: readonly Pick<Hinweis, "stufe">[]): {
  fehler: number;
  warnung: number;
  info: number;
} {
  const z = { fehler: 0, warnung: 0, info: 0 };
  for (const h of hinweise) z[h.stufe] += 1;
  return z;
}

/** Ist die Übergabe an Outlook erlaubt? Nur ein Fehler sperrt (wie `uebergabe_erlaubt` im Crate). */
export function uebergabeErlaubt(
  hinweise: readonly Pick<Hinweis, "stufe">[],
): boolean {
  return !hinweise.some((h) => h.stufe === "fehler");
}

/** Zusammenfassung der Einwilligungen (nur Anzeige). */
export function einwilligungsStand(consents: readonly Consent[]): {
  zugestimmt: number;
  offen: number;
  abgelehnt: number;
} {
  const z = { zugestimmt: 0, offen: 0, abgelehnt: 0 };
  for (const c of consents) z[c.state] += 1;
  return z;
}

/** Anzeigereihenfolge der Personen: Ablehnung zuerst (sperrt), dann offen, dann Zustimmung. */
export function einwilligungenSortiert(
  consents: readonly Consent[],
): Consent[] {
  const rang = { abgelehnt: 0, offen: 1, zugestimmt: 2 } as const;
  return [...consents].sort(
    (a, b) =>
      rang[a.state] - rang[b.state] || a.person.localeCompare(b.person, "de"),
  );
}

/** Klassifizierungen für die Auswahl: die festen plus ein bereits gespeicherter fremder Wert. */
export function klassifizierungen(aktuell: string): string[] {
  const t = aktuell.trim();
  return t === "" || KLASSIFIZIERUNGEN.includes(t)
    ? [...KLASSIFIZIERUNGEN]
    : [...KLASSIFIZIERUNGEN, t];
}
