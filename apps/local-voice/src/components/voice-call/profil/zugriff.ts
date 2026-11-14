/**
 * Der Zugriff auf das Verzeichnis der eigenen Assistenten über die Tauri-Befehle `voice_assistant_*` (Rust:
 * `voice_call/assistenten.rs`). Dünn: Namen und Fehler umsetzen, sonst nichts (die Prüfung steht in `speicher.ts`).
 */
import { commands, type AssistentenFehler } from "@/bindings";
import {
  SpeicherFehler,
  type RohListe,
  type SpeicherCode,
  type Zugriff,
} from "./speicher";

/** Fehler eines Befehls (oder eines Aufrufs, der gar nicht ankam) als `SpeicherFehler`. */
function fehlerVon(e: unknown): SpeicherFehler {
  const code: SpeicherCode =
    typeof e === "string" && e.length > 0
      ? (e as AssistentenFehler)
      : "read_failed";
  return new SpeicherFehler(code);
}

type Antwort<T> =
  { status: "ok"; data: T } | { status: "error"; error: AssistentenFehler };

/** Entpackt die Antwort eines Befehls; ein Fehler (auch ein ausgefallener Aufruf) wird ein `SpeicherFehler`. */
async function ruf<T>(aufruf: () => Promise<Antwort<T>>): Promise<T> {
  let antwort: Antwort<T>;
  try {
    antwort = await aufruf();
  } catch (e) {
    throw fehlerVon(e);
  }
  if (antwort.status === "ok") return antwort.data;
  throw fehlerVon(antwort.error);
}

export const tauriZugriff: Zugriff = {
  async liste(): Promise<RohListe> {
    const l = await ruf(() => commands.voiceAssistantsList());
    return {
      dateien: l.dateien.map((d) => ({
        name: d.name,
        inhalt: d.inhalt,
        abgewiesen: d.abgewiesen,
      })),
      aktivId: l.aktiv_id,
      aktivBelegt: l.aktiv_belegt,
    };
  },
  async speichern(id, inhalt, ersetzt): Promise<void> {
    await ruf(() => commands.voiceAssistantSave(id, inhalt, ersetzt));
  },
  async loeschen(id, erwartet): Promise<void> {
    await ruf(() => commands.voiceAssistantDelete(id, erwartet));
  },
  async aktivSetzen(id): Promise<void> {
    await ruf(() => commands.voiceAssistantActiveSet(id));
  },
};
