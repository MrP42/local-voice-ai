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
  return h ? h(args as never) : null;
}
