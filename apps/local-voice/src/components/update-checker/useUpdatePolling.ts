import { useEffect, useRef, type MutableRefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { pollDue, focusDue } from "./updatePolicy";

/** Eine Uhr fuer Intervall, Fokus und Ruhezustand; manuelle Pruefungen zaehlen mit. */
export function useUpdatePolling(
  enabled: boolean,
  check: () => Promise<void>,
  lastCheck: MutableRefObject<number>,
) {
  const checkRef = useRef(check);
  checkRef.current = check;
  useEffect(() => {
    if (!enabled) return;
    let lastTick = Date.now();
    const run = () => void checkRef.current();
    run();
    const focus = () => {
      if (focusDue(Date.now(), lastCheck.current)) run();
    };
    const timer = setInterval(() => {
      const now = Date.now();
      const due = pollDue(now, lastCheck.current, lastTick);
      lastTick = now;
      if (due) run();
    }, 30_000);
    window.addEventListener("focus", focus);
    const unlisten = listen<boolean>("tauri://focus", ({ payload }) => {
      if (payload) focus();
    });
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", focus);
      void unlisten.then((fn) => fn());
    };
  }, [enabled, lastCheck]);
}
