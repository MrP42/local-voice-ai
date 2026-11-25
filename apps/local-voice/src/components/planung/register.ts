import type { Attendee, Plan, PlanLink } from "@/bindings";
import { istAbgeschlossen, statusMarken, type StatusMarke } from "./logik";

/**
 * Register der Planung (lva#844 U1): Planen · Termin & Zusagen · Durchführen · Protokoll.
 * Reine Logik ohne React: welches Register beim Öffnen gilt und was jedes Register über den
 * Stand sagt. Alles wird aus dem Datenstand abgeleitet (Planung, Verknüpfung, Termin, Lauf),
 * nie aus der gewählten Ansicht; ein Registerwechsel ändert nichts am Stand.
 */

export type RegisterId = "planen" | "termin" | "durchfuehren" | "protokoll";

export const REGISTER: readonly RegisterId[] = [
  "planen",
  "termin",
  "durchfuehren",
  "protokoll",
];

/** Darstellung des Registerpunkts: Haken, Warnung, laufend, offen oder nicht verfügbar. */
export type RegisterStand =
  "offen" | "fertig" | "achtung" | "laeuft" | "gesperrt";

/** Der abgeleitete Satz eines Registers als i18n-Schlüssel (`planung.register.info.<..>`). */
export interface RegisterInfo {
  stand: RegisterStand;
  schluessel: string;
  werte?: Record<string, string | number>;
}

/** Was ein Lauf über sich verrät (ohne Inhalt). */
export interface LaufStand {
  art: "transkription" | "notizen";
  anwesende: number;
}

export interface RegisterDaten {
  plan: Pick<Plan, "status"> | null;
  link: PlanLink | null;
  /** Teilnehmende des verknüpften Termins; `null`, solange keiner gefunden ist. */
  teilnehmende: readonly Pick<Attendee, "partstat">[] | null;
  agendaPunkte: number;
  /** Die Regelprüfung meldet mindestens einen Fehler. */
  regelFehler: boolean;
  /** Ungespeicherte Änderungen im Formular. */
  geaendert: boolean;
  lauf: LaufStand | null;
}

/** Marken des Abgleichs, die im Register „Termin & Zusagen“ eine Handlung verlangen. */
const KALENDER_ACHTUNG: ReadonlySet<StatusMarke["schluessel"]> = new Set([
  "konflikt",
  "verschoben",
  "abgesagt",
  "verwaist",
]);

/** Die erste Kalendermarke, die Aufmerksamkeit verlangt, sonst `null`. */
export function kalenderAchtung(
  plan: Pick<Plan, "status">,
  link: PlanLink | null,
): StatusMarke | null {
  return (
    statusMarken(plan, link).find((m) => KALENDER_ACHTUNG.has(m.schluessel)) ??
    null
  );
}

/**
 * Register beim Öffnen: im Lauf „Durchführen“, bei einer offenen Kalendermeldung (Konflikt,
 * Verschiebung, Absage, Termin fehlt) „Termin & Zusagen“, sonst „Planen“.
 */
export function startRegister(
  plan: Pick<Plan, "status"> | null,
  link: PlanLink | null,
  laeuft: boolean,
): RegisterId {
  if (laeuft) return "durchfuehren";
  if (plan && kalenderAchtung(plan, link)) return "termin";
  return "planen";
}

const partstat = (a: Pick<Attendee, "partstat">) =>
  (a.partstat ?? "").trim().toUpperCase();

/** Zusagen aus dem Termin: zugesagt, geantwortet, eingeladen. */
export function zusagen(teilnehmende: readonly Pick<Attendee, "partstat">[]): {
  zugesagt: number;
  geantwortet: number;
  gesamt: number;
} {
  let zugesagt = 0;
  let geantwortet = 0;
  for (const a of teilnehmende) {
    const p = partstat(a);
    if (p === "ACCEPTED") zugesagt += 1;
    if (p !== "" && p !== "NEEDS-ACTION") geantwortet += 1;
  }
  return { zugesagt, geantwortet, gesamt: teilnehmende.length };
}

/** Kurzes Datum des Termins („Fr., 28.11., 10:00“), immer 24-Stunden-Zeit. */
export function kurzDatum(ms: number): string {
  return new Intl.DateTimeFormat("de-DE", {
    weekday: "short",
    day: "2-digit",
    month: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(ms));
}

function planenInfo(d: RegisterDaten): RegisterInfo {
  if (!d.plan) return { stand: "offen", schluessel: "neu" };
  if (istAbgeschlossen(d.plan.status))
    return { stand: "fertig", schluessel: "abgeschlossen" };
  const werte = { count: d.agendaPunkte };
  if (d.geaendert)
    return { stand: "offen", schluessel: "ungespeichert", werte };
  if (d.regelFehler)
    return { stand: "achtung", schluessel: "unvollstaendig", werte };
  return { stand: "fertig", schluessel: "vollstaendig", werte };
}

function terminInfo(d: RegisterDaten): RegisterInfo {
  if (!d.plan) return { stand: "gesperrt", schluessel: "nachSpeichern" };
  if (d.lauf)
    return {
      stand: "fertig",
      schluessel: "anwesend",
      werte: { count: d.lauf.anwesende },
    };
  const achtung = kalenderAchtung(d.plan, d.link);
  const liste = d.teilnehmende ?? [];
  if (liste.length > 0) {
    const z = zusagen(liste);
    const info: RegisterInfo =
      z.geantwortet === 0
        ? {
            stand: "offen",
            schluessel: "geantwortet",
            werte: { anzahl: 0, count: z.gesamt },
          }
        : {
            stand: z.zugesagt === z.gesamt ? "fertig" : "offen",
            schluessel: "zugesagt",
            werte: { anzahl: z.zugesagt, count: z.gesamt },
          };
    return achtung ? { ...info, stand: "achtung" } : info;
  }
  if (achtung)
    return {
      stand: "achtung",
      schluessel: "marke",
      werte: { marke: achtung.schluessel },
    };
  if (d.link?.starts_at != null)
    return { stand: "fertig", schluessel: "terminSteht" };
  if (d.plan.status === "uebergeben")
    return { stand: "offen", schluessel: "wartetAufTermin" };
  return { stand: "offen", schluessel: "keinTermin" };
}

function durchfuehrenInfo(d: RegisterDaten): RegisterInfo {
  if (!d.plan) return { stand: "gesperrt", schluessel: "nachSpeichern" };
  if (d.lauf)
    return {
      stand: "laeuft",
      schluessel:
        d.lauf.art === "transkription"
          ? "laeuftTranskription"
          : "laeuftNotizen",
    };
  if (istAbgeschlossen(d.plan.status))
    return { stand: "fertig", schluessel: "durchgefuehrt" };
  if (d.link?.starts_at != null)
    return {
      stand: "offen",
      schluessel: "datum",
      werte: { datum: kurzDatum(d.link.starts_at) },
    };
  return { stand: "offen", schluessel: "ohneTermin" };
}

/** Der abgeleitete Satz je Register. Protokoll ist bis #738 nicht verfügbar. */
export function registerInfos(
  d: RegisterDaten,
): Record<RegisterId, RegisterInfo> {
  return {
    planen: planenInfo(d),
    termin: terminInfo(d),
    durchfuehren: durchfuehrenInfo(d),
    protokoll: { stand: "gesperrt", schluessel: "kommtMit738" },
  };
}
