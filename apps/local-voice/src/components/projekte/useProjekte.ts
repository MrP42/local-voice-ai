import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  commands,
  type EintragSicht,
  type LlmConnection,
  type LlmModelConfig,
  type SuiteProjekt,
} from "@/bindings";
import { speakerModelId } from "../voice-call/orchestration";
import { useSettings } from "@/hooks/useSettings";
import { modelLabel } from "@/lib/modelNames";
import { auspacken } from "@/lib/projekte";
import { useProjekteMeldung } from "@/lib/projekteBus";
import { STANDARD_FENSTER } from "@/lib/projekteKontext";

export interface ProjekteStand {
  projekte: SuiteProjekt[];
  /** Eintraege je Projekt: DIE Quelle fuer jede Zahl (Liste links, Kopf der Seite, Leiste). */
  eintraege: Record<string, EintragSicht[]>;
  geladen: boolean;
  fehler: string | null;
  neuLaden: () => Promise<void>;
}

/** Zahl der Eintraege eines Projekts: Laenge von `projektEintraege`, nie ein zweiter Zaehler. */
export const anzahlVon = (s: ProjekteStand, id: string): number =>
  s.eintraege[id]?.length ?? 0;

/**
 * Nicht archivierte Suite-Projekte samt ihren Eintraegen. Lädt bei jeder Meldung `projekte` neu
 * (Zuordnen, Entfernen, Abgleich). Ein veralteter Lauf überschreibt keinen neueren.
 */
export function useProjekteStand(): ProjekteStand {
  const [projekte, setProjekte] = useState<SuiteProjekt[]>([]);
  const [eintraege, setEintraege] = useState<Record<string, EintragSicht[]>>(
    {},
  );
  const [geladen, setGeladen] = useState(false);
  const [fehler, setFehler] = useState<string | null>(null);
  const lauf = useRef(0);

  const neuLaden = useCallback(async () => {
    const mein = ++lauf.current;
    try {
      const liste = auspacken(await commands.projekteListe()).filter(
        (p) => !p.archived,
      );
      const je = await Promise.all(
        liste.map(
          async (p) =>
            [p.id, auspacken(await commands.projektEintraege(p.id))] as const,
        ),
      );
      if (mein !== lauf.current) return;
      setProjekte(liste);
      setEintraege(Object.fromEntries(je));
      setFehler(null);
    } catch (e) {
      if (mein !== lauf.current) return;
      setFehler(e instanceof Error ? e.message : String(e));
    } finally {
      if (mein === lauf.current) setGeladen(true);
    }
  }, []);

  useEffect(() => {
    void neuLaden();
  }, [neuLaden]);
  useProjekteMeldung("projekte", () => void neuLaden());

  return { projekte, eintraege, geladen, fehler, neuLaden };
}

export interface Modellfenster {
  tokens: number;
  /** Anzeigename des Modells; `null`, wenn keines eingerichtet ist. */
  modell: string | null;
}

/** Fenster des Sprechermodells (Sprachgespräch bzw. Standardmodell), sonst 128 000. */
export function useModellfenster(): Modellfenster {
  const { getSetting } = useSettings();
  const connections = (getSetting("llm_connections") ?? []) as LlmConnection[];
  const models = (getSetting("llm_models") ?? []) as LlmModelConfig[];
  const aktiv = speakerModelId({
    function_models: getSetting("function_models"),
    llm_active_model_id: getSetting("llm_active_model_id"),
    llm_models: models,
    llm_connections: connections,
  });
  return useMemo(() => {
    const m = models.find((x) => x.id === aktiv);
    if (!m) return { tokens: STANDARD_FENSTER, modell: null };
    return {
      tokens: m.max_input_tokens ?? m.context_limit ?? STANDARD_FENSTER,
      modell: modelLabel(m.remote_id, m.label),
    };
  }, [models, aktiv]);
}

/**
 * Ist die KI im Projekt freigegeben? Die Sperre ist fest in Rust (`projekt_ki_status`), die Oberfläche
 * schaltet sie nicht. `null`, solange die Antwort aussteht oder der Aufruf scheitert (dann gilt: gesperrt).
 */
export function useProjektKiFreigegeben(): boolean | null {
  const [frei, setFrei] = useState<boolean | null>(null);
  useEffect(() => {
    let aktuell = true;
    commands
      .projektKiStatus()
      .then((s) => {
        if (aktuell) setFrei(s.freigegeben);
      })
      .catch(() => {
        if (aktuell) setFrei(false);
      });
    return () => {
      aktuell = false;
    };
  }, []);
  return frei;
}
