/**
 * Eigene Assistenten laden und auswählen (lva#556, Paket G2). Reine Logik ohne Tauri: der Zugriff auf die Dateien kommt
 * über `Zugriff` herein (im Betrieb die Befehle `voice_assistant_*` aus Rust, im Test ein Speicher im Arbeitsspeicher).
 *
 * Rust hält die Dateien, dieser Teil prüft sie: jede Datei geht durch `parseProfil` (strenger Leser, Fehlerliste), der
 * Dateiname muss zur `id` passen (`muster`, Pfad `id`), vorbehaltene Ids sind abgewiesen (`wert`, Pfad `id`). Eine abgewiesene
 * Datei steht mit ihrer Fehlerliste in `abgewiesen`; sie wird nie repariert, nie überschrieben, nie gelöscht. Vorlagen kommen
 * aus `VORLAGEN`, nie aus dem Speicher. Ist der Speicher nicht lesbar, gibt es nur die Vorlagen und die Auswahl „Alles“;
 * es wird dabei nichts geschrieben.
 */
import {
  ALLES_ID,
  VORLAGEN,
  istVorbehalteneId,
  parseProfil,
  vorlage,
} from "../assistenten";
import type { Profil, ProfilFehler } from "../assistenten";

/** Warum Rust eine Datei nicht als Text geliefert hat (Namen wie die Fehlercodes der Profilprüfung). */
export type Abweisung = "json" | "laenge" | "zu_viele" | "wert";

/** Eine Datei des Verzeichnisses, wie Rust sie liefert. */
export interface RohDatei {
  name: string;
  inhalt: string | null;
  abgewiesen: Abweisung | null;
}

export interface RohListe {
  dateien: RohDatei[];
  /** Die Id in `aktiv.json`, wenn die Datei ein Auswahldokument ist. */
  aktivId: string | null;
  /** `aktiv.json` ist belegt (kein Auswahldokument): Auswählen ist nicht möglich. */
  aktivBelegt: boolean;
}

/** Fehlercodes der Befehle (Rust: `AssistentenFehler`). */
export type SpeicherCode =
  | "invalid_id"
  | "reserved_id"
  | "not_found"
  | "too_large"
  | "too_many"
  | "occupied"
  | "write_failed"
  | "read_failed"
  | "unsafe_path";

export class SpeicherFehler extends Error {
  readonly code: SpeicherCode;
  constructor(code: SpeicherCode) {
    super(code);
    this.name = "SpeicherFehler";
    this.code = code;
  }
}

/** Zugriff auf das Verzeichnis der eigenen Assistenten. Jeder Fehler ist ein `SpeicherFehler`. */
export interface Zugriff {
  liste(): Promise<RohListe>;
  /** Überschreibt (`ersetzt` = der geprüfte alte Text) oder legt neu an (`ersetzt` = `null`). */
  speichern(id: string, inhalt: string, ersetzt: string | null): Promise<void>;
  /** Löscht, wenn die Datei noch genau `erwartet` enthält. */
  loeschen(id: string, erwartet: string): Promise<void>;
  aktivSetzen(id: string): Promise<void>;
}

/** Ein gültiger eigener Assistent mit dem Text, den die Datei hat (für Überschreiben und Löschen). */
export interface EigenerAssistent {
  profil: Profil;
  text: string;
}

export interface AbgewiesenerAssistent {
  datei: string;
  fehler: ProfilFehler[];
}

export interface Assistenten {
  vorlagen: readonly Profil[];
  eigene: EigenerAssistent[];
  abgewiesen: AbgewiesenerAssistent[];
  /** Die wirksame Auswahl: Vorlage oder gültiger eigener Assistent, sonst `alles`. */
  aktivId: string;
  /** `aktiv.json` ist belegt: eine Auswahl lässt sich nicht speichern. */
  aktivBelegt: boolean;
  /** Der Speicher war nicht lesbar (nur Vorlagen, Auswahl „Alles“). */
  ladefehler: SpeicherCode | null;
}

/** Zustand ohne Speicher: nur die Vorlagen, „Alles“. */
export function nurVorlagen(ladefehler: SpeicherCode | null): Assistenten {
  return {
    vorlagen: VORLAGEN,
    eigene: [],
    abgewiesen: [],
    aktivId: ALLES_ID,
    aktivBelegt: false,
    ladefehler,
  };
}

/** Die Abweisung von Rust als Fehlerliste (Pfad leer, wie im Swift-Speicher). */
function rohFehler(grund: Abweisung): ProfilFehler[] {
  return [{ pfad: "", code: grund }];
}

type Gelesen =
  | { ok: true; eigener: EigenerAssistent }
  | { ok: false; fehler: ProfilFehler[] };

/** Prüft eine Datei: lesbar, gültiges Profil, Dateiname = Id, keine vorbehaltene Id. */
function lies(datei: RohDatei): Gelesen {
  if (datei.abgewiesen)
    return { ok: false, fehler: rohFehler(datei.abgewiesen) };
  if (datei.inhalt === null) return { ok: false, fehler: rohFehler("json") };
  const ergebnis = parseProfil(datei.inhalt);
  if (!ergebnis.ok) return { ok: false, fehler: ergebnis.fehler };
  const profil = ergebnis.profil;
  if (`${profil.id}.json` !== datei.name)
    return { ok: false, fehler: [{ pfad: "id", code: "muster" }] };
  if (istVorbehalteneId(profil.id))
    return { ok: false, fehler: [{ pfad: "id", code: "wert" }] };
  return { ok: true, eigener: { profil, text: datei.inhalt } };
}

/**
 * Lädt alle eigenen Assistenten. Ein Lesefehler des Speichers wirft nicht: er steht in `ladefehler`, und es gilt „Alles“
 * (die Auswahl wird dann weder gelesen noch geschrieben).
 */
export async function ladeAssistenten(z: Zugriff): Promise<Assistenten> {
  let roh: RohListe;
  try {
    roh = await z.liste();
  } catch (e) {
    return nurVorlagen(e instanceof SpeicherFehler ? e.code : "read_failed");
  }
  const eigene: EigenerAssistent[] = [];
  const abgewiesen: AbgewiesenerAssistent[] = [];
  for (const datei of roh.dateien) {
    const gelesen = lies(datei);
    if (gelesen.ok) eigene.push(gelesen.eigener);
    else abgewiesen.push({ datei: datei.name, fehler: gelesen.fehler });
  }
  const gewaehlt = roh.aktivId;
  const gueltig =
    gewaehlt !== null &&
    (vorlage(gewaehlt) !== undefined ||
      eigene.some((e) => e.profil.id === gewaehlt));
  return {
    vorlagen: VORLAGEN,
    eigene,
    abgewiesen,
    aktivId: gueltig ? gewaehlt : ALLES_ID,
    aktivBelegt: roh.aktivBelegt,
    ladefehler: null,
  };
}

/** Das Profil zur Id: Vorlage oder gültiger eigener Assistent. */
export function profilZu(a: Assistenten, id: string): Profil | undefined {
  return vorlage(id) ?? a.eigene.find((e) => e.profil.id === id)?.profil;
}

/** Das aktive Profil (nie `undefined`: im Zweifel „Alles“). */
export function aktivesProfil(a: Assistenten): Profil {
  return profilZu(a, a.aktivId) ?? (vorlage(ALLES_ID) as Profil);
}

/**
 * Wählt einen Assistenten aus. Nur eine Vorlage oder ein gültiger eigener Assistent lässt sich wählen (sonst
 * `not_found`); geschrieben wird allein `aktiv.json` (Rust). Ein Fehler ändert nichts an der Auswahl.
 */
export async function waehleAssistent(
  z: Zugriff,
  a: Assistenten,
  id: string,
): Promise<void> {
  if (!profilZu(a, id)) throw new SpeicherFehler("not_found");
  await z.aktivSetzen(id);
}
