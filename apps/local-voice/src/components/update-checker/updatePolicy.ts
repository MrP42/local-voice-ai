export const UPDATE_INTERVAL = 30 * 60_000;
export const UPDATE_FOCUS_INTERVAL = 5 * 60_000;

export function pollDue(now: number, lastCheck: number, lastTick: number): boolean {
  return now - lastTick > 2 * 60_000 || now - lastCheck >= UPDATE_INTERVAL;
}

export function focusDue(now: number, lastCheck: number): boolean {
  return now - lastCheck >= UPDATE_FOCUS_INTERVAL;
}

export function promptDue(
  version: string,
  busy: boolean,
  prompted: ReadonlySet<string>,
  later: { version: string; bis: number } | null,
  now: number,
): boolean {
  return !busy && !prompted.has(version) && !(later?.version === version && later.bis > now);
}

/** Ports halten den Ablauf ohne Tauri und ohne Timer pruefbar. */
export async function waitForUpdateGate(
  attempt: () => Promise<"ok" | "belegt">,
  listenFree: (notify: () => void) => Promise<() => void>,
  signal: AbortSignal,
): Promise<void> {
  let changed = false;
  let wake: (() => void) | undefined;
  const notify = () => {
    changed = true;
    wake?.();
  };
  const unlisten = await listenFree(notify);
  signal.addEventListener("abort", notify);
  try {
    for (;;) {
      if (signal.aborted) throw new Error("update_abgebrochen");
      changed = false;
      if (await attempt() === "ok") return;
      if (!changed && !signal.aborted) {
        await new Promise<void>((resolve) => {
          wake = resolve;
        });
        wake = undefined;
      }
    }
  } finally {
    signal.removeEventListener("abort", notify);
    unlisten();
  }
}
