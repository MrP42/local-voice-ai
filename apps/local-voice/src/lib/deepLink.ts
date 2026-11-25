// TF3: Uebergabe an die Terminfindungsansicht. Ein Link fuehrt keine Fachaktion aus.
export type DeepLinkTarget = { ziel: "terminfindung"; id: string };

let last: DeepLinkTarget | null = null;
const subscribers = new Set<() => void>();

// Fuer useSyncExternalStore; nur das letzte Ziel, ohne Persistenz oder Protokollierung.
export const deepLinkTarget = {
  getSnapshot: () => last,
  subscribe: (listener: () => void) => {
    subscribers.add(listener);
    return () => {
      subscribers.delete(listener);
    };
  },
};

type Listen = (
  event: string,
  handler: (event: { payload: unknown }) => void,
) => Promise<() => void>;

export function connectDeepLinks(listen: Listen, ready: () => Promise<unknown>) {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void listen("deep-link-open", ({ payload }) => {
    if (disposed || !payload || typeof payload !== "object") return;
    const target = payload as Partial<DeepLinkTarget>;
    if (
      target.ziel !== "terminfindung" ||
      typeof target.id !== "string" ||
      !/^T-[A-Z0-9]{4}$/.test(target.id)
    ) return;
    last = { ziel: target.ziel, id: target.id };
    subscribers.forEach((listener) => listener());
  })
    .then(async (stop) => {
      if (disposed) {
        stop();
        return;
      }
      unlisten = stop;
      // Erst nach bestaetigtem Listener darf Rust seinen letzten Eintrag zustellen.
      await ready();
    })
    .catch(() => {
      // Keine Fehlerobjekte aus IPC loggen: sie koennten Nutzlast enthalten.
      unlisten?.();
      unlisten = undefined;
    });
  return () => {
    disposed = true;
    unlisten?.();
    unlisten = undefined;
  };
}
