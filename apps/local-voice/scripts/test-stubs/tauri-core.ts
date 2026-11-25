// Attrappe für @tauri-apps/api/core: jeder Aufruf geht an `globalThis.__tauri.handlers`.
interface Tauri {
  calls: { cmd: string; args: unknown }[];
  handlers: Record<string, (args: never) => unknown>;
}
export class Channel {}
export async function invoke(cmd: string, args?: unknown): Promise<unknown> {
  const t = (globalThis as unknown as { __tauri: Tauri }).__tauri;
  t.calls.push({ cmd, args });
  const h = t.handlers[cmd];
  // Rust meldet die Frist und ruft danach selbst voice_call_cancel auf.
  if (!h && cmd === "voice_call_frist") return invoke("voice_call_cancel", args);
  return h ? h(args as never) : null;
}
