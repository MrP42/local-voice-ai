import { useEffect, useRef, useState } from "react";
import { commands, events, type QueueSnapshot } from "@/bindings";

/**
 * U7: Stand der Import-Warteschlange. Der Zustand liegt im Backend: beim
 * Einhaengen wird er abgefragt, danach ersetzt ihn jedes `importQueueEvent`
 * vollstaendig (verlustfrei: ein verpasstes Ereignis wird vom naechsten
 * ueberholt). `null`, solange nichts bekannt ist (oder ein Backend ohne die
 * Abfrage, z. B. eine Attrappe).
 */
export function useImportQueue(): QueueSnapshot | null {
  const [snapshot, setSnapshot] = useState<QueueSnapshot | null>(null);
  // Ein Ereignis ist neuer als die Antwort der Abfrage und bleibt.
  const eventSeen = useRef(false);

  useEffect(() => {
    let alive = true;
    eventSeen.current = false;
    const un = events.importQueueEvent.listen((e) => {
      eventSeen.current = true;
      setSnapshot(e.payload.snapshot);
    });
    void commands.meetingsQueueList().then((result) => {
      if (!alive || eventSeen.current || result.status !== "ok") return;
      setSnapshot(result.data ?? null);
    });
    return () => {
      alive = false;
      un.then((f) => f());
    };
  }, []);

  return snapshot;
}
