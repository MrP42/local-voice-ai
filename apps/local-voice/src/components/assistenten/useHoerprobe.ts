/**
 * Hörprobe im Editor (lva#556, G2 Teil 2): `HoerprobeBesitzer` (geprüfte Logik in `voice-call/profil/hoerprobe.ts`) an die
 * vorhandene Sprachausgabe des Gesprächs gehängt (`voice_call_synthesize`: die Stimme aus den Einstellungen) und an ein
 * Audioelement. Es gibt EINE Hörprobe für die ganze Seite: alle Knöpfe teilen sich den Besitzer, ein zweiter Start stoppt den
 * ersten, und `voice_call_cancel` bricht eine noch laufende Synthese wirklich ab (die Anfrage-Id geht mit). Läuft ein
 * Gespräch, gibt es keine Hörprobe; beginnt eines, endet alles (auch die Stimmenvorschau).
 */
import { useEffect, useSyncExternalStore } from "react";
import { commands } from "@/bindings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import {
  HoerprobeBesitzer,
  type HoerprobeAbhaengig,
  type HoerprobeZustand,
  type Wiedergabe,
} from "@/components/voice-call/profil/hoerprobe";
import { stoppeVorschau } from "../settings/tts/voices/VoicePreviewButton";

const abhaengig: HoerprobeAbhaengig = {
  gespraechLaeuft: () => useVoiceCallStore.getState().state !== "idle",
  async synthese(text, anfrage) {
    const antwort = await commands.voiceCallSynthesize(text, anfrage);
    if (antwort.status === "error") throw new Error(antwort.error);
    return antwort.data;
  },
  abbrechen: (anfrage) => commands.voiceCallCancel(anfrage),
  spiele(wavBase64): Wiedergabe {
    const audio = new Audio(`data:audio/wav;base64,${wavBase64}`);
    let fertig: () => void = () => undefined;
    let scheitern: (e: Error) => void = () => undefined;
    const ende = new Promise<void>((ok, nein) => {
      fertig = ok;
      scheitern = nein;
    });
    audio.addEventListener("ended", () => fertig());
    audio.addEventListener("error", () => scheitern(new Error("audio")));
    audio
      .play()
      .catch((e: unknown) =>
        scheitern(e instanceof Error ? e : new Error("audio")),
      );
    return {
      ende,
      stopp: () => {
        audio.pause();
        fertig();
      },
    };
  },
};

let besitzer: HoerprobeBesitzer | null = null;

/** Der eine Besitzer der Hörprobe (beim ersten Gebrauch angelegt; er hört auf den Beginn eines Gesprächs). */
export function hoerprobenBesitzer(): HoerprobeBesitzer {
  if (besitzer === null) {
    const neu = new HoerprobeBesitzer(abhaengig, [stoppeVorschau]);
    useVoiceCallStore.subscribe((jetzt, vorher) => {
      if (jetzt.state !== "idle" && vorher.state === "idle")
        neu.gespraechBegonnen();
    });
    besitzer = neu;
  }
  return besitzer;
}

/** Hält die Hörprobe und die Stimmenvorschau an (Wechsel des Eintrags, Verlassen der Seite). */
export function stoppeHoerproben(): void {
  besitzer?.stoppAlles();
}

const BEREIT: HoerprobeZustand = { art: "bereit" };

/**
 * Eine Hörprobe für den Knopf `quelle`: der Zustand ist der der gemeinsamen Hörprobe, solange sie diesem Knopf gehört
 * (sonst „bereit“). Verschwindet der Knopf, endet seine Hörprobe, nie die eines anderen.
 */
export function useHoerprobe(quelle: string) {
  const b = hoerprobenBesitzer();
  const stand = useSyncExternalStore(b.abonniere, () => b.stand);
  const gespraechLaeuft = useVoiceCallStore((s) => s.state !== "idle");
  useEffect(() => () => b.stoppFuer(quelle), [b, quelle]);
  return {
    zustand: stand.quelle === quelle ? stand.zustand : BEREIT,
    gespraechLaeuft,
    spiele: (text: string) => void b.spiele(quelle, text),
    stopp: () => b.stoppFuer(quelle),
  };
}

/**
 * Für die Stimmenvorschau im Editor: sie ist gesperrt, solange ein Gespräch läuft, und beginnt sie, endet die Hörprobe.
 */
export function useVorschauKoordination() {
  const b = hoerprobenBesitzer();
  const gespraechLaeuft = useVoiceCallStore((s) => s.state !== "idle");
  return { gesperrt: gespraechLaeuft, beiStart: () => b.fremdeBeginnt() };
}
