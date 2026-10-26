import { useEffect, useState } from "react";
import { commands } from "@/bindings";

/**
 * Anzeigename eines Anbieters (`provider:<id>` in `tts_engine`). Bis die Liste
 * da ist, steht die Kennung; ohne Anbieter (`id === null`) wird nichts geladen
 * -- die Pruefung der Anbieter kostet einen Programmaufruf und soll nur
 * laufen, wenn ein Anbieter wirklich vorliest.
 */
export function useProviderLabel(id: string | null): string | null {
  const [label, setLabel] = useState<string | null>(null);
  useEffect(() => {
    setLabel(null);
    if (!id) return;
    let cancelled = false;
    void commands
      .ttsProvidersList()
      .then((result) => {
        if (cancelled || result.status !== "ok") return;
        const found = result.data.find((e) => e.descriptor.id === id);
        if (found) setLabel(found.descriptor.label);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [id]);
  return id ? (label ?? id) : null;
}
