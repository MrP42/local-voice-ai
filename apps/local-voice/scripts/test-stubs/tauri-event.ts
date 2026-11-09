// Attrappe für @tauri-apps/api/event: `listen` sammelt die Hörer, der Test sendet sie an.
interface Tauri {
  listeners: ((e: { payload: unknown }) => void)[];
}
export async function listen(
  _name: string,
  cb: (e: { payload: unknown }) => void,
): Promise<() => void> {
  const t = (globalThis as unknown as { __tauri: Tauri }).__tauri;
  t.listeners.push(cb);
  return () => undefined;
}
