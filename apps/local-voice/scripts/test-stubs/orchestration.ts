// Attrappe der Orchestrierung: `globalThis.__tauri.prepare` liefert Seite, Sperre und Systemanweisung.
interface Tauri {
  prepare?: (cfg: { systemPrompt: string }) => unknown;
}
export const DEFAULT_ORCHESTRATION = {};
export const SOUVERAENITAET_STUFEN = ["nur_lokal", "eu", "frei"];
export class OrchestrationSession {
  /** Die Rückrufe der Engine (Test: `ausgabeKey` und Kennung des Gesprächs). */
  readonly deps: Record<string, (...args: never[]) => unknown>;
  constructor(deps: Record<string, (...args: never[]) => unknown> = {}) {
    this.deps = deps;
  }
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
