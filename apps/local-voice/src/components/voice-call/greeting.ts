/**
 * Begrüßung auf der leeren Startseite (lva#394, Vorbild ChatGPT): Tageszeit,
 * Name aus dem aktiven Profil und eine von zwei Varianten, die je Aufruf wechselt.
 * Rein: keine Uhr, kein Zufall, kein Store – beides kommt als Argument.
 */
export type GreetingSlot = "morning" | "day" | "evening" | "night";

export function greetingSlot(hour: number): GreetingSlot {
  if (hour >= 5 && hour < 11) return "morning";
  if (hour >= 11 && hour < 18) return "day";
  if (hour >= 18 && hour < 23) return "evening";
  return "night";
}

/** Ersatznamen, die der Kontospeicher oder die Oberfläche ohne echten Namen setzt (klein geschrieben). */
const ERSATZNAMEN: ReadonlySet<string> = new Set([
  "lokales profil",
  "local profile",
  "profil",
  "profile",
]);

/** Vorname, sonst der erste Teil des Anzeigenamens; leer, wenn nichts Sinnvolles da ist. */
export function greetingName(profil: {
  vorname?: string;
  anzeigename?: string;
}): string {
  const vorname = profil.vorname?.trim();
  if (vorname) return vorname;
  const anzeige = profil.anzeigename?.trim() ?? "";
  // Der Ersatzname des Kontospeichers („Lokales Profil“) ist kein Name: sonst „Guten Abend, Lokales.“
  if (ERSATZNAMEN.has(anzeige.toLowerCase().replace(/\s+/g, " "))) return "";
  const erster = anzeige.split(/\s+/)[0] ?? "";
  // Ein Platzhalter wie „Profil 1“ oder eine Mailadresse ist keine Anrede.
  if (!erster || erster.includes("@") || /^profil$/i.test(erster)) return "";
  return erster;
}

/** Übersetzungsschlüssel und Index der Variante. */
export function greetingKey(
  hour: number,
  name: string,
  seed: number,
): { key: string; index: number } {
  const slot = greetingSlot(hour);
  return {
    key: `voiceCall.greeting.${slot}${name ? "" : "Anon"}`,
    index: Math.abs(Math.trunc(seed)) % 2,
  };
}
