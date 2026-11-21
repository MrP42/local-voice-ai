import type {
  Consent,
  ConsentState,
  EinwilligungsEintrag,
  StartModus,
} from "@/bindings";

export const UNBEKANNT = "unbekannt";

export interface Anwesender {
  id: string;
  email: string | null;
  name: string | null;
}

export function personSchluessel(eingabe: string): string | null {
  const text = eingabe.trim();
  if (/^id:/i.test(text)) {
    const id = text.slice(3).trim();
    return /^[a-z0-9_-]{1,64}$/i.test(id) ? `id:${id}` : null;
  }
  const adresse = /<([^<>]*)>/.exec(text)?.[1] ?? text;
  const wert = adresse
    .trim()
    .replace(/^mailto:/i, "")
    .toLowerCase();
  return /^[^\s@]+@[^\s@.][^\s@]*\.[^\s@.]+$/.test(wert) ? wert : null;
}

export function anwesendeAusTermin(
  teilnehmende: readonly { email: string | null; name: string | null }[],
): Anwesender[] {
  const gesehen = new Set<string>();
  const liste: Anwesender[] = [];
  for (const person of teilnehmende) {
    const email = person.email === null ? null : personSchluessel(person.email);
    if (email !== null) {
      if (gesehen.has(email)) continue;
      gesehen.add(email);
    }
    liste.push({ id: `a${liste.length + 1}`, email, name: person.name });
  }
  return liste;
}

export function anwesenderHinzufuegen(
  liste: readonly Anwesender[],
  eingabe: string,
): { liste: Anwesender[]; fehler: "ungueltig" | "doppelt" | null } {
  const email = personSchluessel(eingabe);
  if (email === null) return { liste: [...liste], fehler: "ungueltig" };
  if (
    liste.some((a) => a.email !== null && personSchluessel(a.email) === email)
  ) {
    return { liste: [...liste], fehler: "doppelt" };
  }
  const nummer =
    liste.reduce((max, a) => {
      const zahl = /\d+$/.exec(a.id);
      return Math.max(max, zahl ? Number(zahl[0]) : 0);
    }, 0) + 1;
  return {
    liste: [...liste, { id: `n${nummer}`, email, name: null }],
    fehler: null,
  };
}

export function anwesendeFuerBackend(liste: readonly Anwesender[]): string[] {
  return liste.map((a) => a.email ?? UNBEKANNT);
}

export function personStand(
  a: Anwesender,
  einwilligungen: readonly Consent[],
): ConsentState | "nicht_identifiziert" {
  if (a.email === null) return "nicht_identifiziert";
  const person = personSchluessel(a.email);
  if (person === null) return "offen";
  return (
    einwilligungen.find((c) => personSchluessel(c.person) === person)?.state ??
    "offen"
  );
}

export function zustandsVerlauf(
  eintraege: readonly EinwilligungsEintrag[],
): EinwilligungsEintrag[] {
  return eintraege
    .map((eintrag, index) => ({ eintrag, index }))
    .sort(
      (a, b) =>
        b.eintrag.festgehalten_am - a.eintrag.festgehalten_am ||
        b.index - a.index,
    )
    .map(({ eintrag }) => eintrag);
}

/** Die Gründe, aus denen der Start nur Notizen zulässt (Namen nur für offen/abgelehnt). */
export function startGruende(modus: StartModus): {
  nichtGeplant: boolean;
  offen: string[];
  abgelehnt: string[];
  nichtIdentifiziert: number;
} {
  if (modus.modus === "mit_transkription") {
    return {
      nichtGeplant: false,
      offen: [],
      abgelehnt: [],
      nichtIdentifiziert: 0,
    };
  }
  return {
    nichtGeplant: modus.nicht_geplant,
    offen: modus.offen.filter((p) => p !== UNBEKANNT),
    abgelehnt: [...modus.abgelehnt],
    nichtIdentifiziert: modus.offen.filter((p) => p === UNBEKANNT).length,
  };
}
