/**
 * Kopie aus @wai/ui konto (wai-sdk PR #991, ADR-0024). Nicht vom Original abweichen; Änderungen dort machen
 * und neu kopieren. Abweichungen hier: `Icon` -> lucide-react (siehe `Icon.tsx`), `cx` -> `cx.ts`, `cleanDisplayName`/`hasUnsafeChars` lokal mitkopiert.
 */

// --- lokal mitkopiert aus wai-sdk packages/ui/src/personen/person.ts und verfassen/adresse.ts ---
const MAX_NAME = 200;

/** Zeilenumbrüche, Steuerzeichen, Zeilen-/Absatztrenner und Bidi-Steuerzeichen: nie in Namen. */
function hasUnsafeChars(s: string): boolean {
  return (
    /[\u0000-\u001f\u007f]/.test(s) ||
    /[\u0080-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/.test(
      s,
    )
  );
}

/** Entfernt Steuer-, Bidi- und Trennzeichen (CR/LF) sowie `<>,;`; leer → `undefined`. */
export function cleanDisplayName(
  name: string | undefined | null,
): string | undefined {
  if (typeof name !== "string") return undefined;
  const stripped = Array.from(name.replace(/[\r\n\t\u2028\u2029]+/g, " "))
    .filter((c) => !hasUnsafeChars(c) && !"<>,;".includes(c))
    .join("")
    .replace(/\s+/g, " ")
    .trim();
  return stripped
    ? Array.from(stripped).slice(0, MAX_NAME).join("")
    : undefined;
}
// --- Ende der lokalen Kopie ---

/**
 * Anzeigemodell des Suite-Profils (ADR-0024). Die Daten kommen vom Host aus `wai-konto`; die Bausteine speichern
 * nichts selbst und laden nie von einer URL (Bild nur als Blob).
 */
export interface KontoProfil {
  id: string;
  anzeigename: string;
  vorname?: string;
  nachname?: string;
  email?: string;
  telefon?: string;
  mobil?: string;
  position?: string;
  abteilung?: string;
  organisation?: string;
  standort?: string;
  /** Initialenfarbe 0–15 (Fluent-Avatar-Palette). */
  farbe?: number;
  /** Profilbild als Blob vom Host (PNG, wie `wai-konto` speichert; JPEG/WebP zur Anzeige erlaubt). */
  bild?: Blob;
  /** Profil ist mit PIN gesperrt (ADR-0024 E4). */
  gesperrt?: boolean;
  /** Gesetzt, wenn das Profil mit einem Anmeldedienst verbunden ist. */
  anmeldung?: KontoAnmeldung;
}

export interface KontoAnmeldung {
  /** Anzeigename des Anmeldedienstes, z. B. „konto.wolffappliedai.de“. */
  anbieter: string;
  /** Organisation laut Anmeldedienst (Kopfzeile des Profilmenüs). */
  organisation?: string;
  seitMs?: number;
  /** Felder, die vom Anmeldedienst kommen und hier schreibgeschützt sind. */
  idpFelder: readonly KontoFeld[];
}

/** `lokal` = nicht angemeldet, `abgelaufen` = Anmeldung muss erneuert werden (lokal geht alles weiter). */
export type Anmeldestatus = "lokal" | "angemeldet" | "abgelaufen";

export type KontoFeld =
  | "anzeigename"
  | "vorname"
  | "nachname"
  | "email"
  | "telefon"
  | "mobil"
  | "position"
  | "abteilung"
  | "organisation"
  | "standort";

export type ProfilAenderung = Partial<Record<KontoFeld, string>>;

export const KONTO_FELDER: readonly KontoFeld[] = [
  "anzeigename",
  "vorname",
  "nachname",
  "email",
  "telefon",
  "mobil",
  "position",
  "abteilung",
  "organisation",
  "standort",
];

export const MAX_BILD_BYTES = 256 * 1024;
const BILD_TYPEN = new Set(["image/png", "image/jpeg", "image/webp"]);

export function istBrauchbaresBild(b: Blob | undefined): b is Blob {
  return (
    !!b && BILD_TYPEN.has(b.type) && b.size > 0 && b.size <= MAX_BILD_BYTES
  );
}

/** Name für Anzeige und Initialen; fällt auf „Lokales Profil“ zurück. */
export function profilName(p: KontoProfil, ersatz = "Lokales Profil"): string {
  return cleanDisplayName(p.anzeigename) ?? ersatz;
}

/** Initialen aus Vor-/Nachname oder Anzeigename; `undefined`, wenn nichts Brauchbares da ist. */
export function profilInitialen(p: KontoProfil): string | undefined {
  const v = cleanDisplayName(p.vorname);
  const n = cleanDisplayName(p.nachname);
  if (v && n) return (Array.from(v)[0] + Array.from(n)[0]).toUpperCase();
  const name = cleanDisplayName(p.anzeigename);
  if (!name) return undefined;
  const teile = name.split(" ").filter((t) => /\p{L}/u.test(t));
  if (teile.length === 0) return undefined;
  const erst = Array.from(teile[0])[0];
  const letzt = teile.length > 1 ? Array.from(teile[teile.length - 1])[0] : "";
  return (erst + letzt).toUpperCase();
}

/** Wie `wai-konto`: gesperrt, solange das Profil mit dem Anmeldedienst verbunden ist (Abmelden leert `anmeldung`). */
export function feldGesperrt(
  p: KontoProfil,
  _status: Anmeldestatus,
  feld: KontoFeld,
): boolean {
  return !!p.anmeldung?.idpFelder.includes(feld);
}

/** Nur geänderte, nicht gesperrte Felder; Werte getrimmt. */
export function aenderungen(
  p: KontoProfil,
  status: Anmeldestatus,
  entwurf: ProfilAenderung,
): ProfilAenderung {
  const out: ProfilAenderung = {};
  for (const f of KONTO_FELDER) {
    const neu = entwurf[f];
    if (neu === undefined || feldGesperrt(p, status, f)) continue;
    const wert = neu.replace(/\s+/g, " ").trim();
    if (wert !== (p[f] ?? "")) out[f] = wert;
  }
  return out;
}

const FARBEN = [
  "dark-red",
  "cranberry",
  "red",
  "pumpkin",
  "peach",
  "marigold",
  "gold",
  "brass",
  "brown",
  "forest",
  "seafoam",
  "dark-green",
  "light-teal",
  "teal",
  "steel",
  "blue",
] as const;
export type AvatarFarbe = (typeof FARBEN)[number];

export function avatarFarbe(p: KontoProfil): AvatarFarbe {
  const i = Number.isInteger(p.farbe) ? (p.farbe as number) : 15;
  return FARBEN[((i % 16) + 16) % 16];
}
