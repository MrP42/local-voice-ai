/**
 * Hörprobe im Editor (lva#556, G2 Teil 2): `Hoerprobe` (geprüfte Logik in `voice-call/profil/hoerprobe.ts`) an die vorhandene
 * Sprachausgabe des Gesprächs gehängt (`voice_call_synthesize`: die Stimme aus den Einstellungen) und an ein Audioelement.
 * Läuft ein Gespräch, gibt es keine Hörprobe.
 */
import { useEffect, useRef, useState } from "react";
import { commands } from "@/bindings";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import {
  Hoerprobe,
  type HoerprobeAbhaengig,
  type HoerprobeZustand,
  type Wiedergabe,
} from "@/components/voice-call/profil/hoerprobe";

const abhaengig: HoerprobeAbhaengig = {
  gespraechLaeuft: () => useVoiceCallStore.getState().state !== "idle",
  async synthese(text) {
    const antwort = await commands.voiceCallSynthesize(text, null);
    if (antwort.status === "error") throw new Error(antwort.error);
    return antwort.data;
  },
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

export function useHoerprobe() {
  const [zustand, setZustand] = useState<HoerprobeZustand>({ art: "bereit" });
  const ref = useRef<Hoerprobe | null>(null);
  if (ref.current === null) ref.current = new Hoerprobe(abhaengig, setZustand);
  const gespraechLaeuft = useVoiceCallStore((s) => s.state !== "idle");
  useEffect(() => {
    const probe = ref.current;
    return () => probe?.stopp();
  }, []);
  // Beginnt ein Gespräch, endet eine laufende Hörprobe (beide teilten sich Sprachdienst und Ausgabe).
  useEffect(() => {
    if (gespraechLaeuft) ref.current?.stopp();
  }, [gespraechLaeuft]);
  return {
    zustand,
    gespraechLaeuft,
    spiele: (text: string) => void ref.current?.starte(text),
    stopp: () => ref.current?.stopp(),
  };
}
