/**
 * Eigene Assistenten verwalten (lva#556, G2 Teil 2): die reine Logik hinter der Seite „Assistenten“. Anlegen, Kopieren,
 * Umbenennen, Export und Import eines Profils, dazu die Wahl einer freien Id. Nichts davon schreibt: das tut
 * `verwaltung.ts` über den `Zugriff`; hier entstehen nur geprüfte Profile und Dateitexte.
 *
 * Regeln (wie im Kern, `apps/shared/assistenten/README.md`):
 *  - Jedes Profil geht durch den Kern (`profilAusBaum`, `pruefeProfil`, `kodiereProfil`); es wird nie still repariert, jede
 *    Abweisung kommt als Fehlerliste zurück. Unbekannte Felder bleiben erhalten, weil alle Änderungen am typisierten Profil
 *    (mit seinen `extra`-Feldern) arbeiten und über den JSON-Baum geprüft werden.
 *  - Die Id ist der Dateiname (`<id>.json`). Sie ändert sich beim Umbenennen nicht; nur der Name ändert sich.
 *  - Eine Id gilt als belegt, wenn es sie als Vorlage, als gültigen eigenen Assistenten oder als Datei gibt, die der Speicher
 *    abgewiesen hat (er überschreibt sie nie, auch wenn Windows und macOS Groß- und Kleinschreibung nicht trennen).
 */
import {
  istGeraetename,
  istVorbehalteneId,
  alsEigenenKopieren,
  kodiereProfil,
  neuesProfil,
  parseProfil,
  profilAusBaum,
  profilBaum,
} from "../assistenten";
import type { Profil, ProfilErgebnis, ProfilFehler } from "../assistenten";
import type { Assistenten } from "./speicher";

/** So viele Dateien nimmt der Speicher höchstens an (Rust: `MAX_DATEIEN`); darüber gilt `too_many`. */
export const MAX_DATEIEN = 200;
/** So groß ist eine Datei höchstens in Bytes (Rust: `MAX_BYTES`); größere lehnt der Speicher und damit der Import ab. */
export const MAX_DATEI_BYTES = 256 * 1024;
/** Höchstlänge einer Id (Muster `^[a-z0-9][a-z0-9-]{0,63}$`). */
const MAX_ID = 64;
/** Höchstlänge des Namens in Unicode-Zeichen (Kern). */
export const MAX_NAME = 60;

// ---------------------------------------------------------------- Ids

const UMLAUTE: ReadonlyArray<readonly [string, string]> = [
  ["ä", "ae"],
  ["ö", "oe"],
  ["ü", "ue"],
  ["ß", "ss"],
];

/**
 * Die Id, die ein Name nahelegt: Kleinbuchstaben und Ziffern, Umlaute als `ae`/`oe`/`ue`/`ss`, andere Akzente ohne Akzent,
 * alles übrige als ein Bindestrich; höchstens 64 Zeichen. Bleibt nichts übrig, ist es `assistent`. Ob sie frei ist, sagt
 * `freieId`.
 */
export function idAusName(name: string): string {
  let s = name.toLowerCase();
  for (const [von, nach] of UMLAUTE) s = s.split(von).join(nach);
  s = s.normalize("NFD").replace(/[̀-ͯ]/g, "");
  s = s.replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
  s = s.slice(0, MAX_ID).replace(/-+$/, "");
  return s === "" ? "assistent" : s;
}

const vorsatz = (basis: string): string =>
  `eigen-${basis}`.slice(0, MAX_ID).replace(/-+$/, "");

const unbrauchbar = (id: string): boolean =>
  istVorbehalteneId(id) || istGeraetename(id);

/**
 * Alle Ids (kleingeschrieben), die ein neuer Assistent nicht bekommen darf: die der eigenen, die Namen der abgewiesenen
 * Dateien ohne Endung (sie bleiben unberührt) und `aktiv`. Die Vorlagen und `vorlage-…` fängt `istVorbehalteneId` ab.
 */
export function belegteIds(a: Assistenten): Set<string> {
  const ids = new Set<string>(["aktiv", "alles"]);
  for (const e of a.eigene) ids.add(e.profil.id.toLowerCase());
  for (const x of a.abgewiesen) {
    const name = x.datei.toLowerCase();
    ids.add(name.endsWith(".json") ? name.slice(0, -".json".length) : name);
  }
  return ids;
}

/**
 * Eine freie Id zu `grund`: ist sie belegt, vorbehalten oder ein Gerätename, kommt `eigen-` davor (nur bei vorbehaltenen und
 * Gerätenamen) und bei Belegung `-2`, `-3`, … dahinter. Das Ergebnis erfüllt das Idmuster und ist nie vorbehalten.
 */
export function freieId(grund: string, belegt: ReadonlySet<string>): string {
  let basis = grund === "" ? "assistent" : grund;
  if (unbrauchbar(basis)) basis = vorsatz(basis);
  let id = basis;
  for (let n = 2; belegt.has(id.toLowerCase()) || unbrauchbar(id); n += 1) {
    const endung = `-${n}`;
    id = `${basis.slice(0, MAX_ID - endung.length).replace(/-+$/, "")}${endung}`;
  }
  return id;
}

/** Ist noch Platz für eine weitere Datei? (Der Speicher nimmt höchstens `MAX_DATEIEN` an; abgewiesene zählen mit.) */
export function hatPlatz(a: Assistenten): boolean {
  return a.eigene.length + a.abgewiesen.length < MAX_DATEIEN;
}

// ---------------------------------------------------------------- Anlegen, Kopieren, Umbenennen

/** Der Name ohne Leerzeichen an den Enden, wie er gespeichert wird. */
export const sauberName = (name: string): string => name.trim();

const nameFehler = (): ProfilErgebnis => ({
  ok: false,
  fehler: [{ pfad: "name", code: "laenge" }],
});

/** Ein neuer, leerer Assistent mit diesem Namen und einer freien Id (geprüft; sonst die Fehlerliste, meist `name`). */
export function neuAnlegen(name: string, a: Assistenten): ProfilErgebnis {
  const sauber = sauberName(name);
  const id = freieId(idAusName(sauber), belegteIds(a));
  const ergebnis = profilAusBaum(profilBaum(neuesProfil(id, sauber)));
  return ergebnis.ok ? ergebnis : nameFehler();
}

/**
 * Eine Kopie von `quelle` (Vorlage oder eigener Assistent) unter neuem Namen und freier Id. Die Kopie teilt nichts mit der
 * Quelle; unbekannte Felder der Quelle bleiben erhalten.
 */
export function kopieren(
  quelle: Profil,
  neuerName: string,
  a: Assistenten,
): ProfilErgebnis {
  const sauber = sauberName(neuerName);
  const id = freieId(idAusName(sauber), belegteIds(a));
  return alsEigenenKopieren(quelle, id, sauber);
}

/** Dasselbe Profil mit anderem Namen (die Id bleibt); geprüft, ohne zu schreiben. */
export function umbenennen(p: Profil, neuerName: string): ProfilErgebnis {
  return profilAusBaum(profilBaum({ ...p, name: sauberName(neuerName) }));
}

// ---------------------------------------------------------------- Export und Import

export interface Ausgabe {
  /** Vorgeschlagener Dateiname (`<id>.json`). */
  dateiname: string;
  /** Die Bytes als Text: genau das, was der Kern schreibt (`kodiereProfil`), UTF-8, mit Zeilenende. */
  text: string;
}

/** Das Profil als Datei, im Format des Kerns (gleiche Bytes wie auf dem iPhone). */
export function exportieren(p: Profil): Ausgabe {
  return { dateiname: `${p.id}.json`, text: kodiereProfil(p) };
}

export type ImportErgebnis =
  | {
      ok: true;
      profil: Profil;
      /** Die Id der Datei war belegt oder vorbehalten: das Profil hat eine neue eigene Id bekommen. */
      idGeaendert: boolean;
      /** Die Id in der Datei. */
      ursprungsId: string;
    }
  | {
      ok: false;
      /** `zu_gross` (über 256 KiB), `leer` (nichts drin) oder `abgewiesen` (der Kern lehnt es ab: `fehler`). */
      grund: "zu_gross" | "leer" | "abgewiesen";
      fehler: ProfilFehler[];
    };

const BYTES = new TextEncoder();

/** Warum Dateibytes nicht als Text gelten: zu groß, mit Byte-Order-Marke (BOM) oder kein gültiges UTF-8. */
export type DekodierGrund = "zu_gross" | "bom" | "kein_utf8";

export type Dekodiert =
  | { ok: true; text: string }
  | { ok: false; grund: DekodierGrund; fehler: ProfilFehler[] };

/**
 * Macht aus den Bytes einer Datei den Text, den der Kern prüft, ohne etwas zu reparieren: `File.text()` würde eine BOM
 * stillschweigend entfernen und ungültiges UTF-8 durch U+FFFD ersetzen, die Datei würde dann als etwas gelesen, das sie
 * nicht ist. Hier gilt: mehr als 256 KiB wird nicht gelesen (`zu_gross`), eine UTF-8-BOM am Anfang wird ausdrücklich
 * abgelehnt (`bom`; der Kern schreibt und liest keine), ungültiges UTF-8 wird abgelehnt (`kein_utf8`). Alles andere kommt
 * Zeichen für Zeichen unverändert durch.
 */
export function dekodiereDatei(bytes: Uint8Array): Dekodiert {
  if (bytes.length > MAX_DATEI_BYTES)
    return { ok: false, grund: "zu_gross", fehler: [] };
  if (
    bytes.length >= 3 &&
    bytes[0] === 0xef &&
    bytes[1] === 0xbb &&
    bytes[2] === 0xbf
  )
    return { ok: false, grund: "bom", fehler: [] };
  try {
    const text = new TextDecoder("utf-8", {
      fatal: true,
      ignoreBOM: true,
    }).decode(bytes);
    return { ok: true, text };
  } catch {
    return { ok: false, grund: "kein_utf8", fehler: [] };
  }
}

/**
 * Liest ein Profil aus Dateitext. Der Kern prüft streng (`parseProfil`); jede Abweisung kommt als Liste zurück, nichts wird
 * repariert. Ist die Id belegt (eigener Assistent, abgewiesene Datei), vorbehalten (Vorlage, `vorlage-…`, `aktiv`) oder ein
 * Gerätename, bekommt das Profil eine neue eigene Id: Der Import überschreibt nie einen vorhandenen Assistenten und fragt
 * nicht, denn der Inhalt geht dabei nicht verloren (umbenennen und löschen geht danach jederzeit).
 */
export function importieren(text: string, a: Assistenten): ImportErgebnis {
  if (BYTES.encode(text).length > MAX_DATEI_BYTES)
    return { ok: false, grund: "zu_gross", fehler: [] };
  if (text.trim() === "") return { ok: false, grund: "leer", fehler: [] };
  const gelesen = parseProfil(text);
  if (!gelesen.ok)
    return { ok: false, grund: "abgewiesen", fehler: gelesen.fehler };
  const profil = gelesen.profil;
  const belegt = belegteIds(a);
  if (!belegt.has(profil.id.toLowerCase()) && !unbrauchbar(profil.id))
    return { ok: true, profil, idGeaendert: false, ursprungsId: profil.id };
  // Aus einer vorbehaltenen Id (Vorlagen-Export) wird die Id aus dem Namen, sonst hängt die nächste freie Zahl an.
  const grund = unbrauchbar(profil.id) ? idAusName(profil.name) : profil.id;
  const neu = alsEigenenKopieren(profil, freieId(grund, belegt), profil.name);
  if (!neu.ok) return { ok: false, grund: "abgewiesen", fehler: neu.fehler };
  return {
    ok: true,
    profil: neu.profil,
    idGeaendert: true,
    ursprungsId: profil.id,
  };
}
