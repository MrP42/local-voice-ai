import { useEffect, useState } from "react";
import { commands } from "@/bindings";

/**
 * Namen der Sprecher, die die Vorlese-Pipeline per Marker `<Name>` schaltet
 * (`tts_provider_speakers`, dieselbe Liste wie der Editor). Einmal je Sitzung
 * geladen; ohne Anbieter mit Stimmwechsel leer.
 */
let cache: Promise<string[]> | null = null;

export function loadSpeakers(): Promise<string[]> {
  cache ??= commands
    .ttsProviderSpeakers()
    .then((r) => {
      if (r.status !== "ok" || !Array.isArray(r.data)) throw new Error("tts");
      return r.data.map((s) => s.name);
    })
    .catch(() => {
      // Kern noch nicht bereit: nicht dauerhaft leer merken, später neu fragen.
      cache = null;
      return [] as string[];
    });
  return cache;
}

export function useSpeakers(): string[] {
  const [names, setNames] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    void loadSpeakers().then((n) => alive && setNames(n));
    return () => {
      alive = false;
    };
  }, []);
  return names;
}
