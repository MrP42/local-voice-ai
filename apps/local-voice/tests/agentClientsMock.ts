import type { Page } from "@playwright/test";

/**
 * Attrappe der Zugangsliste der Agentenbrücke (A7, #66) für `integrations-agents.spec.ts`.
 * Sie legt sich über `installTauriMock` und `installIntegrationsMock` (Register) und
 * übernimmt die `agent_client_*`-Kommandos und `agent_bridge_status`. Es gibt weder Pipe
 * noch Token-Prüfung; die Rechenregel der wirksamen Rechte spiegelt `bridge::tool_rights`
 * (Obergrenze der Agent-Integration und Recht des Zugangs, das Strengere gewinnt; „Aufnahme
 * starten“ nie erlaubt). Die echten Regeln prüfen die Rust-Tests (`cargo test --lib agent_bridge::`).
 *
 * Die Zugänge stehen im localStorage (`lva.test.agents`): Neuladen behält sie, wie die
 * Datenbank es im echten Programm tut. Das Token kennt nur die Antwort von
 * `agent_client_create`; danach liest es nichts mehr aus.
 */

export interface MockAgentClient {
  id: string;
  label: string;
  integration_id?: string;
  created_at?: number;
  last_used_at?: number | null;
  revoked_at?: number | null;
  /** Werkzeug -> Recht des Zugangs (ohne Eintrag: off). */
  tools?: Record<string, string>;
}

export interface AgentClientsMockOptions {
  clients?: MockAgentClient[];
  /** Zustand der Pipe (Standard: läuft). */
  bridge?: {
    running: boolean;
    pipe_name?: string | null;
    error?: string | null;
    /** A8: Pfad der Programmdatei (fertige Anbindungsbefehle). */
    exe_path?: string | null;
  };
  /** Werkzeuge, die diese App-Version ausführen kann (Standard: zwei). */
  available?: string[];
}

export const PIPE = "\\\\.\\pipe\\local-voice-ai-agent-0123456789abcdef";

export const installAgentClientsMock = async (
  page: Page,
  options: AgentClientsMockOptions = {},
) => {
  await page.addInitScript(
    ({ options, pipe }) => {
      const w = window as any;
      const KEY = "lva.test.agents";
      const NOW = 1_790_000_000_000;
      const TOOLS = [
        ["add_youtube_source", "youtube.add"],
        ["start_recording", "recording.start"],
        ["stop_recording", "recording.start"],
        ["transcribe_file", "transcribe.file"],
        ["create_session", "meeting.create"],
        ["create_meeting", "meeting.create"],
        ["tts_page_create", "tts.render"],
        ["tts_render_audio", "tts.render"],
      ];
      const ALPHABET =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
      const ORDER: Record<string, number> = { off: 0, ask: 1, allow: 2 };

      let state: any = null;
      try {
        const stored = window.localStorage.getItem(KEY);
        if (stored) state = JSON.parse(stored);
      } catch {
        /* ohne Speicher: frischer Zustand */
      }
      if (!state) {
        state = {
          next: 1,
          clients: (options.clients ?? []).map((c: any) => ({
            integration_id: "agent-1",
            created_at: NOW - 86_400_000,
            last_used_at: null,
            revoked_at: null,
            tools: {},
            ...c,
          })),
        };
      }
      w.__agents = state;
      const save = () => {
        try {
          window.localStorage.setItem(KEY, JSON.stringify(state));
        } catch {
          /* egal */
        }
      };
      const available = new Set(
        options.available ?? ["transcribe_file", "create_meeting"],
      );
      const ceiling = (integrationId: string, cap: string) => {
        const integ = (w.__reg?.integrations ?? []).find(
          (i: any) => i.id === integrationId,
        );
        if (!integ) return { mode: "off", reason: "integration_disabled" };
        if (!integ.enabled)
          return { mode: "off", reason: "integration_disabled" };
        const stored = integ.grants?.[`${cap}|agent_external`] ?? "off";
        return { mode: stored, reason: stored === "off" ? "grant_off" : null };
      };
      const toView = (c: any) => ({
        client: {
          id: c.id,
          label: c.label,
          integration_id: c.integration_id,
          created_at: c.created_at,
          last_used_at: c.last_used_at,
          revoked_at: c.revoked_at,
        },
        tools: TOOLS.map(([name, cap]) => {
          const mine = c.tools[name] ?? "off";
          const up = ceiling(c.integration_id, cap);
          let effective = mine;
          let reason: string | null = null;
          if (up.mode === "off") {
            effective = "off";
            reason = up.reason;
          } else {
            if (ORDER[mine] < ORDER[up.mode]) effective = mine;
            else effective = up.mode;
            if (effective === "off") reason = "tool_off";
            if (cap === "recording.start" && effective === "allow") {
              effective = "ask";
            }
          }
          return {
            name,
            title: `Titel ${name}`,
            description: `Beschreibung ${name}`,
            capability: cap,
            client_mode: mine,
            effective_mode: effective,
            off_reason: reason,
            available: available.has(name),
          };
        }),
      });
      const find = (id: string) => {
        const c = state.clients.find((x: any) => x.id === id);
        if (!c) throw `Integration nicht gefunden: ${id}`;
        return c;
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        switch (cmd) {
          case "agent_client_list":
            w.__calls.push({ cmd, args });
            return state.clients.map(toView);
          case "agent_bridge_status":
            w.__calls.push({ cmd, args });
            return (
              options.bridge ?? {
                running: true,
                pipe_name: pipe,
                error: null,
                exe_path: "C:\Program Files\Local Voice AI\local-voice-ai.exe",
              }
            );
          case "agent_client_create": {
            w.__calls.push({ cmd, args });
            const label = String(args.label ?? "").trim();
            if (!label) throw "Der Name des Zugangs fehlt.";
            if (state.clients.filter((c: any) => !c.revoked_at).length >= 20) {
              throw "Es gibt schon 20 aktive Zugänge. Bitte zuerst einen zurückziehen.";
            }
            const n = state.next++;
            const client = {
              id: `01MOCKCLIENT${String(n).padStart(4, "0")}`,
              label,
              integration_id: (args.integrationId as string) ?? "agent-1",
              created_at: NOW,
              last_used_at: null,
              revoked_at: null,
              tools: {},
            };
            state.clients.push(client);
            save();
            const token =
              "lvat_" +
              Array.from(
                { length: 43 },
                (_, i) => ALPHABET[(i * 7 + n * 3) % ALPHABET.length],
              ).join("");
            return { client: toView(client).client, token };
          }
          case "agent_client_revoke": {
            w.__calls.push({ cmd, args });
            const c = find(args.id as string);
            if (!c.revoked_at) c.revoked_at = NOW;
            save();
            return toView(c).client;
          }
          case "agent_client_delete": {
            w.__calls.push({ cmd, args });
            find(args.id as string);
            state.clients = state.clients.filter((c: any) => c.id !== args.id);
            save();
            return null;
          }
          case "agent_client_set_tool_mode": {
            w.__calls.push({ cmd, args });
            const c = find(args.clientId as string);
            const known = TOOLS.some(([name]) => name === args.tool);
            if (!known) throw "Dieses Werkzeug gibt es nicht.";
            if (
              (args.tool === "start_recording" ||
                args.tool === "stop_recording") &&
              args.mode === "allow"
            ) {
              throw "Aufnahme starten kann nie dauerhaft erlaubt werden.";
            }
            c.tools[args.tool as string] = args.mode;
            save();
            return toView(c);
          }
          default:
            return original(cmd, args);
        }
      };
    },
    { options, pipe: PIPE },
  );
};
