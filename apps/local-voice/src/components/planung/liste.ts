import type { Plan, PlanLink } from "@/bindings";
import { filtere, type Bereich } from "../ui/auswahlSuche";
import {
  listenGruppe,
  statusMarken,
  type ListenGruppe,
  type StatusMarke,
} from "./logik";

/**
 * Liste der Planungen (lva#844 U2): Suche, Zeitgruppen, ein Zustandschip je Zeile und die
 * Zähler der Filter. Reine Logik ohne React; alles wird aus Planung und Verknüpfung abgeleitet.
 */

export type ListenFilter = ListenGruppe | "alle";

export const FILTER: readonly ListenFilter[] = ["aktiv", "archiv", "alle"];

/** Zeitgruppe einer Zeile; Reihenfolge wie in der sortierten Liste. */
export type ZeitGruppe =
  "frueher" | "dieseWoche" | "naechsteWoche" | "spaeter" | "ohneTermin";

/** Montag 00:00 (Ortszeit des Geräts) der Woche, in der `jetzt` liegt. */
export function wochenAnfang(jetzt: number): number {
  const d = new Date(jetzt);
  d.setHours(0, 0, 0, 0);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  return d.getTime();
}

/** Ortszeit-Mitternacht `tage` Kalendertage nach `anfang` (sommerzeitfest). */
function plusTage(anfang: number, tage: number): number {
  const d = new Date(anfang);
  d.setDate(d.getDate() + tage);
  return d.getTime();
}

/**
 * Gruppe nach dem Beginn des Termins, gemessen an der Kalenderwoche (Montag bis Sonntag) von
 * `jetzt`: früher, diese Woche, nächste Woche, später; ohne Beginn „ohne Termin“.
 */
export function zeitGruppe(
  beginn: number | null | undefined,
  jetzt: number,
): ZeitGruppe {
  if (beginn == null || !Number.isFinite(beginn)) return "ohneTermin";
  const montag = wochenAnfang(jetzt);
  if (beginn < montag) return "frueher";
  if (beginn < plusTage(montag, 7)) return "dieseWoche";
  if (beginn < plusTage(montag, 14)) return "naechsteWoche";
  return "spaeter";
}

const TON_RANG: Record<StatusMarke["ton"], number> = {
  danger: 0,
  warning: 1,
  info: 2,
  success: 2,
  neutral: 2,
};

/**
 * Der eine Zustandschip einer Zeile: was Handlung verlangt (Konflikt, Absage, Verschiebung,
 * Termin fehlt) vor dem Status der Planung. Abgeleitet aus `statusMarken`, nie behauptet.
 */
export function listenMarke(
  plan: Pick<Plan, "status">,
  link: PlanLink | null,
): StatusMarke {
  const marken = statusMarken(plan, link);
  let beste = marken[0];
  for (const m of marken.slice(1))
    if (m.ton !== "info" && TON_RANG[m.ton] < TON_RANG[beste.ton]) beste = m;
  return beste;
}

/** Trennzeichen zwischen den Feldern: ein Treffer kann nicht über ein Feld hinausreichen. */
const FELDGRENZE = "\n";

/**
 * Wonach die Liste sucht: Titel, Ziel, erwartetes Ergebnis, Agenda (Thema und Verantwortliche),
 * Rollen, Unterlagen, Klassifizierung und Ort des Termins. Personen des Kalenders kennt die
 * Liste nicht (sie lädt keine Termine).
 */
export function suchText(plan: Plan, link: PlanLink | null): string {
  const felder: (string | null | undefined)[] = [
    plan.title,
    plan.goal_text,
    plan.expected_result,
    plan.classification,
    plan.roles?.moderation,
    plan.roles?.ergebnissicherung,
    plan.roles?.entscheidung,
    link?.location,
  ];
  for (const a of plan.agenda ?? []) felder.push(a.topic, a.owner);
  for (const u of plan.materials ?? []) felder.push(u.label);
  return felder
    .map((f) => (f ?? "").replace(/\s+/g, " ").trim())
    .filter((f) => f !== "")
    .join(FELDGRENZE);
}

/** Trifft der Suchtext die Planung (Suchvertrag aus #839, `auswahlSuche`)? */
export function trifft(
  plan: Plan,
  link: PlanLink | null,
  suche: string,
): boolean {
  if (suche.trim() === "") return true;
  return filtere([{ label: suchText(plan, link) }], suche).length > 0;
}

/** Fundstelle im Titel (für die Hervorhebung), sonst `null`. */
export function titelTreffer(titel: string, suche: string): Bereich | null {
  if (suche.trim() === "") return null;
  return filtere([{ label: titel }], suche)[0]?.bereich ?? null;
}

export interface ListenZeile {
  plan: Plan;
  link: PlanLink | null;
  gruppe: ZeitGruppe;
  marke: StatusMarke;
  /** Der Termin hat einen Beitrittslink (aus Outlook). */
  online: boolean;
}

export interface ListenStand {
  /** Sichtbare Zeilen nach Filter und Suche, in der Reihenfolge der Liste. */
  zeilen: ListenZeile[];
  /** Zeilen je Gruppe (aufeinanderfolgend, leere Gruppen fehlen). */
  gruppen: { gruppe: ZeitGruppe; zeilen: ListenZeile[] }[];
  /** Treffer je Filter unter der aktuellen Suche. */
  zaehler: Record<ListenFilter, number>;
  /** Aktive Planungen ohne Suche (Kopfzeile „6 aktiv“). */
  aktivGesamt: number;
  /** Planungen im Archiv ohne Suche. */
  archivGesamt: number;
}

/**
 * Leitet den ganzen Stand der Liste in einem Durchgang ab. `plaene` ist schon sortiert
 * (`sortiere`); Gruppen bleiben deshalb zusammenhängend.
 */
export function listenStand(
  plaene: readonly Plan[],
  links: ReadonlyMap<string, PlanLink | null>,
  filter: ListenFilter,
  suche: string,
  jetzt: number,
): ListenStand {
  const zaehler: Record<ListenFilter, number> = {
    aktiv: 0,
    archiv: 0,
    alle: 0,
  };
  let aktivGesamt = 0;
  let archivGesamt = 0;
  const zeilen: ListenZeile[] = [];
  for (const plan of plaene) {
    const link = links.get(plan.id) ?? null;
    const art = listenGruppe(plan, link);
    if (art === "aktiv") aktivGesamt += 1;
    else archivGesamt += 1;
    if (!trifft(plan, link, suche)) continue;
    zaehler[art] += 1;
    zaehler.alle += 1;
    if (filter !== "alle" && filter !== art) continue;
    zeilen.push({
      plan,
      link,
      gruppe: zeitGruppe(link?.starts_at, jetzt),
      marke: listenMarke(plan, link),
      online: !!link?.join_url?.trim(),
    });
  }
  const gruppen: ListenStand["gruppen"] = [];
  for (const z of zeilen) {
    const letzte = gruppen[gruppen.length - 1];
    if (letzte && letzte.gruppe === z.gruppe) letzte.zeilen.push(z);
    else gruppen.push({ gruppe: z.gruppe, zeilen: [z] });
  }
  return { zeilen, gruppen, zaehler, aktivGesamt, archivGesamt };
}

/** Datum einer Zeile („Mi., 28.10., 10:00“), mit Jahr nur außerhalb des laufenden Jahres. */
export function zeilenDatum(
  ms: number,
  sprache: string,
  jetzt: number,
): string {
  const mitJahr = new Date(ms).getFullYear() !== new Date(jetzt).getFullYear();
  try {
    return new Intl.DateTimeFormat(sprache, {
      weekday: "short",
      day: "2-digit",
      month: "2-digit",
      year: mitJahr ? "numeric" : undefined,
      hour: "2-digit",
      minute: "2-digit",
      hourCycle: "h23",
    }).format(new Date(ms));
  } catch {
    return new Date(ms).toLocaleString();
  }
}
