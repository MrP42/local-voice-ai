// Attrappe der Orchestrierung: `globalThis.__tauri.prepare` liefert Seite, Sperre und Systemanweisung.
interface Tauri {
  prepare?: (cfg: { systemPrompt: string }) => unknown;
}
export const DEFAULT_ORCHESTRATION = {};
export class OrchestrationSession {
  dispose(): void {}
  invalidate(): void {}
  async prepare(cfg: { systemPrompt: string }): Promise<unknown> {
    const t = (globalThis as unknown as { __tauri: Tauri }).__tauri;
    return (
      (await t.prepare?.(cfg)) ?? {
        side: null,
        blocked: false,
        systemPrompt: cfg.systemPrompt,
      }
    );
  }
}
