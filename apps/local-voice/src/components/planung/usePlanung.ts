import { useCallback, useEffect, useRef, useState } from "react";
import type { Plan, PlanLink } from "@/bindings";
import { api } from "./api";
import { sortiere } from "./logik";

export interface PlanungDaten {
  plaene: Plan[];
  links: ReadonlyMap<string, PlanLink | null>;
  laden: boolean;
  /** Der Fehlercode der letzten Ladung, sonst `null`. */
  fehler: string | null;
  neuLaden: () => Promise<void>;
}

/** Liste der Planungen samt Verknüpfung je Planung (für Termin und Status in der Liste). */
export function usePlanung(): PlanungDaten {
  const [plaene, setPlaene] = useState<Plan[]>([]);
  const [links, setLinks] = useState<ReadonlyMap<string, PlanLink | null>>(
    new Map(),
  );
  const [laden, setLaden] = useState(true);
  const [fehler, setFehler] = useState<string | null>(null);
  const lauf = useRef(0);
  const lebt = useRef(true);
  useEffect(() => {
    lebt.current = true;
    return () => {
      lebt.current = false;
    };
  }, []);

  const neuLaden = useCallback(async () => {
    const mein = ++lauf.current;
    try {
      const liste = await api.liste();
      const paare = await Promise.all(
        liste.map(async (p) => {
          try {
            return [p.id, await api.verknuepfung(p.id)] as const;
          } catch {
            return [p.id, null] as const;
          }
        }),
      );
      if (!lebt.current || mein !== lauf.current) return;
      const map = new Map(paare);
      setLinks(map);
      setPlaene(sortiere(liste, map));
      setFehler(null);
    } catch (e) {
      if (!lebt.current || mein !== lauf.current) return;
      setFehler(e instanceof Error ? e.message : String(e));
    } finally {
      if (lebt.current && mein === lauf.current) setLaden(false);
    }
  }, []);

  useEffect(() => {
    void neuLaden();
  }, [neuLaden]);

  return { plaene, links, laden, fehler, neuLaden };
}
