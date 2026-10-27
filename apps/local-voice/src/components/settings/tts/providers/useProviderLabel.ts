import { useEffect, useState } from "react";
import { commands } from "@/bindings";

/**
 * Was die Oberflaeche ueber den Anbieter wissen muss, der gerade vorliest: sein
 * Anzeigename und zwei Faehigkeiten (`ProviderCaps`). Daran haengt, ob Sprecher im
 * Skript gelten und ob Stil-Tags angeboten werden -- nicht an der Kennung des Anbieters.
 */
export interface ProviderInfo {
  label: string;
  /** Spricht je Satz eine andere Stimme: Marker `<Name>` schalten (`voice_switching`). */
  voiceSwitching: boolean;
  /** Versteht Stil-Tags `[...]` (`style_tags`); sonst wirken nur Pausen. */
  styleTags: boolean;
}

type ProviderList = Awaited<ReturnType<typeof commands.ttsProvidersList>>;

/* Gleichzeitige Leser teilen sich EINEN Aufruf: die Pruefung der Anbieter kostet einen
   Programmaufruf (Gesundheit), und die Vorlesen-Seite fragt von mehreren Stellen. Ein
   spaeterer Aufruf liest neu, wie bisher. */
let inFlight: Promise<ProviderList> | null = null;
const loadProviders = (): Promise<ProviderList> => {
  if (!inFlight) {
    inFlight = commands.ttsProvidersList().finally(() => {
      inFlight = null;
    });
  }
  return inFlight;
};

/**
 * Beschreibung des Anbieters (`provider:<id>` in `tts_engine`); `null` bis die Liste da
 * ist und ohne Anbieter (`id === null`) -- dann wird nichts geladen: die Pruefung der
 * Anbieter soll nur laufen, wenn ein Anbieter wirklich vorliest.
 */
export function useProviderInfo(id: string | null): ProviderInfo | null {
  const [info, setInfo] = useState<ProviderInfo | null>(null);
  useEffect(() => {
    setInfo(null);
    if (!id) return;
    let cancelled = false;
    void loadProviders()
      .then((result) => {
        if (cancelled || result.status !== "ok") return;
        const found = result.data.find((e) => e.descriptor.id === id);
        if (found) {
          setInfo({
            label: found.descriptor.label,
            voiceSwitching: found.descriptor.caps.voice_switching === true,
            styleTags: found.descriptor.caps.style_tags === true,
          });
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [id]);
  return info;
}

/**
 * Anzeigename eines Anbieters (`provider:<id>` in `tts_engine`). Bis die Liste
 * da ist, steht die Kennung.
 */
export function useProviderLabel(id: string | null): string | null {
  const info = useProviderInfo(id);
  return id ? (info?.label ?? id) : null;
}
