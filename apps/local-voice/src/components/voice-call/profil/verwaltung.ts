/**
 * Eigene Assistenten schreiben (lva#556, G2 Teil 2): Anlegen, Überschreiben und Löschen über den `Zugriff`. Reine Logik ohne
 * Tauri; `verwalten.ts` liefert die geprüften Profile, hier steht die Reihenfolge der Schritte und die Abbildung der Fehler.
 *
 * Fehlerfälle und wie sie abgesichert sind:
 *  1. Nebenläufigkeit (zwei Fenster, iPhone-Abgleich, Doppelklick): Rust überschreibt nur, wenn die Datei noch genau den
 *     geprüften alten Text hat (`ersetzt`), und legt neue Dateien nur an, wenn es sie nicht gibt (`occupied`). Der Store
 *     reiht alle Schreibvorgänge hintereinander (`schreiben`). Ein Konflikt bricht ab; der Entwurf bleibt erhalten.
 *  2. Abbruch mitten im Vorgang: geschrieben wird atomar (Rust: tmp, fsync, rename); hier gibt es nie zwei Schritte, deren
 *     halber Stand Daten kostet. Beim Löschen des gewählten Assistenten wird zuerst die Auswahl auf „Alles“ gesetzt und dann
 *     die Datei gelöscht: bricht es dazwischen ab, bleibt der Assistent erhalten, und ein Rest in `aktiv.json` fällt beim
 *     Lesen auf „Alles“ zurück (`ladeAssistenten`).
 *  3. Voller Datenträger oder Schreibfehler: `write_failed`; die alte Datei bleibt unverändert, der Entwurf bleibt im Editor.
 *  4. Fehlendes oder unsicheres Verzeichnis: der Speicher meldet `read_failed`/`unsafe_path` beim Lesen; es wird dann nichts
 *     geschrieben (jede Funktion prüft `ladefehler`), und die Seite zeigt den Grund.
 *  5. Datei ist inzwischen anders, gelöscht oder belegt: `occupied` und `not_found` kommen unverändert zurück. Beim Anlegen
 *     mit selbst gewählter Id (Neu, Kopie, Import) wird nach `occupied` neu gelesen und mit der nächsten freien Id wiederholt
 *     (höchstens `VERSUCHE` mal), damit ein Abgleich im selben Moment weder überschrieben noch als Fehler gezeigt wird.
 *  6. Entwurf auf altem Stand: wartet „Speichern“ auf ein Nachladen und die Datei hat sich inzwischen geändert, kennt der
 *     Speicher schon den fremden Text. Verglichen wird deshalb gegen die Originalbytes, auf denen der Entwurf begann
 *     (`erwartet` bei `ueberschreibe`), und diese gehen als Bedingung bis zum Schreibbefehl: Konflikt (`occupied`), nichts
 *     wird geschrieben, die fremde Änderung bleibt.
 *
 * Grenze dieses Schutzes: Der Vergleich ist nur **innerhalb des Prozesses** atomar (Rust hält eine Sperre vom Vergleich
 * bis nach dem Umbenennen), nicht prozessübergreifend im Dateisystem. Ein anderer Prozess am selben Verzeichnis (zweite
 * Instanz, Abgleich, Editor) kann zwischen Vergleich und Umbenennen schreiben und wird dabei überschrieben; dagegen gilt
 * die Regel „Ein Schreiber je Verzeichnis“ (`apps/shared/assistenten/README.md`), nicht diese Bedingung.
 */
import { ALLES_ID, kodiereProfil, pruefeProfil } from "../assistenten";
import type { Profil, ProfilErgebnis, ProfilFehler } from "../assistenten";
import {
  SpeicherFehler,
  type Assistenten,
  type SpeicherCode,
  type Zugriff,
} from "./speicher";
import { MAX_DATEI_BYTES, belegteIds, hatPlatz } from "./verwalten";

/** So oft versucht das Anlegen mit einer neuen Id, wenn die gewählte inzwischen belegt ist. */
export const VERSUCHE = 3;

/** Warum etwas nicht geschrieben wurde: ein Befehlsfehler des Speichers oder eine Abweisung der Prüfung. */
export type Fehlergrund =
  | SpeicherCode
  /** Das Profil besteht die Prüfung des Kerns nicht (`fehler` nennt die Felder). */
  | "ungueltig"
  /** Import: der Kern lehnt die Datei ab (`fehler` nennt die Felder). */
  | "abgewiesen"
  /** Import: die Datei ist größer als 256 KiB. */
  | "zu_gross"
  /** Import: die Datei ist leer. */
  | "leer"
  /** Import: die Datei beginnt mit einer Byte-Order-Marke (UTF-8-BOM); der Kern schreibt und liest keine. */
  | "bom"
  /** Import: die Datei ist kein gültiges UTF-8. */
  | "kein_utf8";

export type Misserfolg = {
  ok: false;
  grund: Fehlergrund;
  fehler: ProfilFehler[];
};
export type Ausgang<T extends object = object> =
  ({ ok: true } & T) | Misserfolg;

const nein = (grund: Fehlergrund, fehler: ProfilFehler[] = []): Misserfolg => ({
  ok: false,
  grund,
  fehler,
});

const bytes = new TextEncoder();

/** Der Grund, warum gar nicht geschrieben wird: der Speicher war nicht lesbar. */
function ohneSpeicher(a: Assistenten): Misserfolg | null {
  return a.ladefehler ? nein(a.ladefehler) : null;
}

/** Ein `SpeicherFehler` (oder ein anderer Ausfall) als Misserfolg. */
function ausFehler(e: unknown): Misserfolg {
  return nein(e instanceof SpeicherFehler ? e.code : "write_failed");
}

/** Prüft das Profil (Kern) und seine Größe; liefert den Text, der geschrieben würde. */
function vorbereiten(p: Profil): { text: string } | Misserfolg {
  const fehler = pruefeProfil(p);
  if (fehler.length > 0) return nein("ungueltig", fehler);
  const text = kodiereProfil(p);
  if (bytes.encode(text).length > MAX_DATEI_BYTES) return nein("too_large");
  return { text };
}

/**
 * Legt `profil` als neue Datei an. Die Id muss frei sein (ohne Rücksicht auf Groß- und Kleinschreibung: Windows und macOS
 * trennen sie nicht), es muss Platz sein, das Profil muss die Prüfung bestehen.
 */
export async function legeAn(
  z: Zugriff,
  a: Assistenten,
  profil: Profil,
): Promise<Ausgang> {
  const kein = ohneSpeicher(a);
  if (kein) return kein;
  if (belegteIds(a).has(profil.id.toLowerCase())) return nein("occupied");
  if (!hatPlatz(a)) return nein("too_many");
  const vor = vorbereiten(profil);
  if ("ok" in vor) return vor;
  try {
    await z.speichern(profil.id, vor.text, null);
  } catch (e) {
    return ausFehler(e);
  }
  return { ok: true };
}

/** Das Ergebnis eines Erzeugers: ein geprüftes Profil samt Zusatzangaben oder die Abweisung. */
export type Erzeugt<T extends object> =
  | ({ ok: true; profil: Profil } & T)
  | Misserfolg
  | Extract<ProfilErgebnis, { ok: false }>;

/**
 * Erzeugt ein neues Profil gegen den aktuellen Stand und legt es an. Ist die Id inzwischen belegt (`occupied`: ein anderer
 * Zugang war schneller), wird neu gelesen und mit einer frischen Id wiederholt, höchstens `VERSUCHE` mal.
 */
export async function legeNeuAn<T extends object = object>(
  z: Zugriff,
  start: Assistenten,
  neuLesen: () => Promise<Assistenten>,
  erzeuge: (a: Assistenten) => Erzeugt<T>,
): Promise<Ausgang<{ profil: Profil } & T>> {
  let a = start;
  for (let versuch = 1; ; versuch += 1) {
    const e = erzeuge(a);
    if (!e.ok) {
      return "grund" in e ? e : nein("ungueltig", e.fehler);
    }
    const { profil } = e;
    const r = await legeAn(z, a, profil);
    if (r.ok) return e as { ok: true; profil: Profil } & T;
    if (r.grund !== "occupied" || versuch >= VERSUCHE) return r;
    a = await neuLesen();
  }
}

/**
 * Speichert die geänderte Fassung eines eigenen Assistenten. Die Id ändert sich nie. Überschrieben wird nur, wenn die Datei
 * noch genau den Text hat, auf dem die Änderung begann (sonst `occupied`: jemand war schneller). Ändert sich nichts, wird
 * nichts geschrieben.
 *
 * `erwartet` ist der Text der Datei, auf dem der Entwurf begann (die Originalbytes der Basis). Er ist die Vergleichsbasis,
 * nicht der Stand des Speichers zum Zeitpunkt des Schreibens: wartet der Vorgang auf ein Nachladen und die Datei hat sich
 * inzwischen geändert, wäre der Stand `a` schon der fremde, und der alte Entwurf überschriebe die fremde Änderung. So
 * kommt es zum Konflikt, nichts wird geschrieben, und `erwartet` geht als Bedingung bis zum Schreibbefehl (Rust vergleicht
 * noch einmal gegen die Datei selbst). Ohne `erwartet` (Umbenennen aus der Liste) gilt der Stand `a`, auf dem der Vorgang
 * selbst aufbaut.
 */
export async function ueberschreibe(
  z: Zugriff,
  a: Assistenten,
  id: string,
  neu: Profil,
  erwartet?: string,
): Promise<Ausgang> {
  const kein = ohneSpeicher(a);
  if (kein) return kein;
  const alt = a.eigene.find((e) => e.profil.id === id);
  if (!alt) return nein("not_found");
  if (neu.id !== id) return nein("ungueltig", [{ pfad: "id", code: "wert" }]);
  const vor = vorbereiten(neu);
  if ("ok" in vor) return vor;
  if (vor.text === alt.text) return { ok: true };
  const basis = erwartet ?? alt.text;
  if (basis !== alt.text) return nein("occupied");
  try {
    await z.speichern(id, vor.text, basis);
  } catch (e) {
    return ausFehler(e);
  }
  return { ok: true };
}

/**
 * Löscht einen eigenen Assistenten (die Datei geht nur, wenn sie noch genau den bekannten Text hat). Ist er der gewählte,
 * geht die Auswahl zuerst auf „Alles“: so bleibt nach keinem Abbruch eine Auswahl auf einen fehlenden Assistenten stehen, und
 * eine später neu angelegte Datei gleicher Id wird nicht heimlich wieder der gewählte. Scheitert die Auswahl, bleibt alles.
 */
export async function loesche(
  z: Zugriff,
  a: Assistenten,
  id: string,
): Promise<Ausgang> {
  const kein = ohneSpeicher(a);
  if (kein) return kein;
  const alt = a.eigene.find((e) => e.profil.id === id);
  if (!alt) return nein("not_found");
  if (a.aktivId === id && !a.aktivBelegt) {
    try {
      await z.aktivSetzen(ALLES_ID);
    } catch (e) {
      return ausFehler(e);
    }
  }
  try {
    await z.loeschen(id, alt.text);
  } catch (e) {
    return ausFehler(e);
  }
  return { ok: true };
}
