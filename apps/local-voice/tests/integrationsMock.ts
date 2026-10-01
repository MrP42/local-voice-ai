import type { Page } from "@playwright/test";

/**
 * Attrappe der Seite „Integrationen“ (A4, #66) fuer `integrations.spec.ts`.
 * Sie legt sich ueber `calendarMock.installTauriMock` (Kalender, Einstellungen)
 * und uebernimmt die Register-Kommandos: Liste, Anlegen, Aendern, Rechte,
 * Test, Protokoll, Freigaben.
 *
 * Das Register steht im localStorage des Testbrowsers (`lva.test.reg`): ein
 * Neuladen der Seite behaelt es, wie die Datenbank es im echten Programm tut.
 * Die Rechenregel der wirksamen Rechte spiegelt `grants::explain` (E3); die
 * echte Regel pruefen die Rust-Tests (`integrations::view`).
 */

export const NOW = 1_790_000_000_000;

export interface MockApproval {
  id: string;
  caller: string;
  integration_id: string | null;
  tool_or_capability: string;
  args_preview: string | null;
  created_at?: number;
}

export interface MockAudit {
  id: number;
  ts: number;
  caller: string;
  integration_id: string | null;
  capability: string | null;
  target: string | null;
  outcome: "ok" | "denied" | "error" | "pending";
  detail_json: string | null;
}

export interface MockIntegration {
  id: string;
  kind: string;
  label: string;
  enabled?: boolean;
  direction?: "read" | "write" | "both";
  path?: string;
  last_error?: string | null;
  /** "<faehigkeit>|<aufrufer>" -> Modus */
  grants?: Record<string, string>;
}

export interface IntegrationsMockOptions {
  integrations?: MockIntegration[];
  approvals?: MockApproval[];
  audit?: MockAudit[];
  /** Ergebnis von `integration_test` (Standard: ok). */
  testCode?: string;
  /** Pfad, den der Ordnerdialog liefert. */
  pickedPath?: string;
}

export const installIntegrationsMock = async (
  page: Page,
  options: IntegrationsMockOptions = {},
) => {
  await page.addInitScript(
    ({ options, now }) => {
      const w = window as any;
      const KEY = "lva.test.reg";
      const KINDS: Record<
        string,
        { directions: string[]; caps: string[]; managed?: boolean }
      > = {
        folder: {
          directions: ["read", "write", "both"],
          caps: ["files.read", "files.write"],
        },
        ics: { directions: ["read"], caps: ["calendar.read"], managed: true },
        smtp: { directions: ["write"], caps: ["mail.send"] },
        youtube: { directions: ["read"], caps: ["media.fetch", "youtube.add"] },
        agent: {
          directions: ["read", "write", "both"],
          caps: [
            "meeting.create",
            "recording.start",
            "transcribe.file",
            "tts.render",
            "youtube.add",
          ],
        },
      };
      const READS = new Set([
        "calendar.read",
        "files.read",
        "knowledge.search",
        "knowledge.read",
        "media.fetch",
      ]);
      const CALLERS = ["workflow", "agent_external", "agent_local"];

      let state: any = null;
      try {
        const stored = window.localStorage.getItem(KEY);
        if (stored) state = JSON.parse(stored);
      } catch {
        /* ohne Speicher: frischer Zustand */
      }
      if (!state) {
        state = {
          nextAudit: 100,
          nextId: 1,
          integrations: (options.integrations ?? []).map((i: any) => ({
            enabled: true,
            direction: KINDS[i.kind]?.directions.length === 1
              ? KINDS[i.kind].directions[0]
              : "both",
            path: null,
            last_error: null,
            grants: {},
            ...i,
          })),
          approvals: (options.approvals ?? []).map((a: any) => ({
            created_at: now - 120_000,
            state: "pending",
            ...a,
          })),
          audit: options.audit ?? [],
        };
      }
      w.__reg = state;
      w.__regCalls = [];
      const save = () => {
        try {
          window.localStorage.setItem(KEY, JSON.stringify(state));
        } catch {
          /* egal */
        }
      };
      const defaultMode = (cap: string, caller: string) =>
        caller === "agent_external"
          ? "off"
          : cap === "media.fetch"
            ? "off"
            : READS.has(cap)
              ? "allow"
              : "ask";
      const permits = (direction: string, cap: string) =>
        direction === "both" ||
        (direction === "read" && READS.has(cap)) ||
        (direction === "write" && !READS.has(cap));
      const toIntegration = (i: any) => ({
        id: i.id,
        kind: i.kind,
        label: i.label,
        enabled: i.enabled,
        direction: i.direction,
        config_json: JSON.stringify(i.path ? { path: i.path } : {}),
        account_hint: null,
        data_class: null,
        created_at: now,
        updated_at: now,
        last_ok_at: null,
        last_error: i.last_error,
      });
      const toView = (i: any) => {
        const kind = KINDS[i.kind] ?? KINDS.folder;
        return {
          integration: toIntegration(i),
          directions: kind.directions,
          capabilities: kind.caps.map((cap) => ({
            capability: cap,
            writes: !READS.has(cap),
            never_allow: cap === "recording.start",
            direction_allows: permits(i.direction, cap),
            modes: CALLERS.map((caller) => {
              const stored = i.grants[`${cap}|${caller}`] ?? null;
              const def = defaultMode(cap, caller);
              let effective = stored ?? def;
              let reason: string | null = null;
              if (!i.enabled) {
                effective = "off";
                reason = "integration_disabled";
              } else if (!permits(i.direction, cap)) {
                effective = "off";
                reason = "direction_blocks";
              } else if (effective === "off") {
                reason = "grant_off";
              } else if (cap === "recording.start" && effective === "allow") {
                effective = "ask";
              }
              return {
                caller,
                stored,
                default_mode: def,
                effective,
                off_reason: reason,
              };
            }),
          })),
          secrets: [],
          calendar_managed: !!kind.managed,
          pending_approvals: state.approvals.filter(
            (a: any) => a.state === "pending" && a.integration_id === i.id,
          ).length,
        };
      };
      const audit = (
        integration_id: string | null,
        capability: string | null,
        detail: Record<string, unknown>,
      ) => {
        state.audit.unshift({
          id: state.nextAudit++,
          ts: now + state.nextAudit * 1000,
          caller: "user",
          integration_id,
          capability,
          target: null,
          outcome: "ok",
          detail_json: JSON.stringify(detail),
        });
      };
      const find = (id: string) => {
        const i = state.integrations.find((x: any) => x.id === id);
        if (!i) throw `Integration nicht gefunden: ${id}`;
        return i;
      };

      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        switch (cmd) {
          case "integrations_list":
            w.__calls.push({ cmd, args });
            return state.integrations.map(toView);
          case "integration_create": {
            w.__calls.push({ cmd, args });
            if (args.kind !== "folder") throw "kind_not_available";
            const path = String(args.path ?? "").trim();
            if (!path) throw "folder_path_missing";
            if (!/^[A-Za-z]:[\\/]/.test(path)) throw "folder_path_relative";
            if (path.includes("gibt-es-nicht")) throw "folder_path_not_found";
            const label = String(args.label ?? "").trim();
            if (!label) throw "Der Name darf nicht leer sein.";
            const i = {
              id: `int-${state.nextId++}`,
              kind: "folder",
              label,
              enabled: true,
              direction: args.direction ?? "both",
              path,
              last_error: null,
              grants: {},
            };
            state.integrations.push(i);
            audit(i.id, null, { phase: "created", kind: "folder" });
            save();
            return toView(i);
          }
          case "integration_update": {
            w.__calls.push({ cmd, args });
            const i = find(args.id as string);
            if (args.label != null) i.label = args.label;
            if (args.enabled != null) i.enabled = args.enabled;
            if (args.direction != null) i.direction = args.direction;
            audit(i.id, null, { phase: "updated" });
            save();
            return toView(i);
          }
          case "integration_delete": {
            w.__calls.push({ cmd, args });
            find(args.id as string);
            state.integrations = state.integrations.filter(
              (x: any) => x.id !== args.id,
            );
            audit(args.id as string, null, { phase: "deleted" });
            save();
            return null;
          }
          case "integration_set_grant": {
            w.__calls.push({ cmd, args });
            const i = find(args.id as string);
            const key = `${args.capability}|${args.caller}`;
            if (args.capability === "recording.start" && args.mode === "allow") {
              throw "Aufnahme starten kann nie dauerhaft erlaubt werden.";
            }
            if (args.mode == null) delete i.grants[key];
            else i.grants[key] = args.mode;
            audit(i.id, args.capability as string, {
              phase: "grant_changed",
              for: args.caller,
              mode: args.mode ?? "default",
            });
            save();
            return toView(i);
          }
          case "integration_test": {
            w.__calls.push({ cmd, args });
            find(args.id as string);
            const code = options.testCode ?? "folder_ok";
            return { ok: code === "folder_ok", code };
          }
          case "integrations_audit_list": {
            w.__calls.push({ cmd, args });
            let rows = state.audit as any[];
            if (args.integrationId)
              rows = rows.filter((r) => r.integration_id === args.integrationId);
            if (args.outcome) rows = rows.filter((r) => r.outcome === args.outcome);
            if (args.caller) rows = rows.filter((r) => r.caller === args.caller);
            return rows.slice(0, Number(args.limit ?? 200));
          }
          case "approvals_pending": {
            const pending = state.approvals.filter(
              (a: any) => a.state === "pending",
            );
            return pending.map((a: any) => {
              const i = state.integrations.find(
                (x: any) => x.id === a.integration_id,
              );
              return {
                approval: {
                  id: a.id,
                  created_at: a.created_at,
                  caller: a.caller,
                  integration_id: a.integration_id,
                  tool_or_capability: a.tool_or_capability,
                  args_preview: a.args_preview,
                  state: "pending",
                  decided_at: null,
                },
                integration_label: i ? i.label : null,
                integration_kind: i ? i.kind : null,
              };
            });
          }
          case "approval_decide": {
            w.__calls.push({ cmd, args });
            const a = state.approvals.find((x: any) => x.id === args.id);
            if (!a) throw "approval_not_found";
            if (a.state !== "pending") throw "approval_already_decided";
            a.state = args.approve ? "approved" : "denied";
            audit(a.integration_id, a.tool_or_capability, {
              phase: "approval_decided",
              decision: args.approve ? "approved" : "denied",
              approval_id: a.id,
              requested_by: a.caller,
            });
            save();
            return {
              id: a.id,
              created_at: a.created_at,
              caller: a.caller,
              integration_id: a.integration_id,
              tool_or_capability: a.tool_or_capability,
              args_preview: a.args_preview,
              state: a.state,
              decided_at: now,
            };
          }
          case "plugin:dialog|open":
            return options.pickedPath ?? "C:\\Ablage\\Berichte";
          // MCP (wie in meeting-mcp.spec.ts)
          case "meeting_mcp_info":
            return { exe_path: "C:\\Program Files\\Local Voice AI\\local-voice-ai.exe" };
          case "change_meeting_mcp_enabled_setting":
            w.__calls.push({ cmd, args });
            w.__settings.meeting_mcp_enabled = args.enabled;
            return null;
          case "change_meeting_mcp_include_transcript_setting":
            w.__calls.push({ cmd, args });
            w.__settings.meeting_mcp_include_transcript = args.enabled;
            return null;
        }
        return original(cmd, args);
      };
    },
    { options, now: NOW },
  );
};

/** Ein demo-Register fuer Screenshots und Pruefungen. */
export const DEMO: IntegrationsMockOptions = {
  integrations: [
    {
      id: "ordner-berichte",
      kind: "folder",
      label: "Ablage Berichte",
      direction: "both",
      path: "C:\\Ablage\\Berichte",
      grants: {
        "files.write|agent_external": "ask",
        "files.read|agent_external": "allow",
      },
    },
    {
      id: "ordner-archiv",
      kind: "folder",
      label: "Archiv (nur lesen)",
      direction: "read",
      path: "D:\\Archiv",
      last_error: "Der Ordner wurde nicht gefunden.",
    },
    {
      id: "yt-1",
      kind: "youtube",
      label: "YouTube",
      direction: "read",
    },
  ],
  approvals: [
    {
      id: "ap-1",
      caller: "agent_external",
      integration_id: "ordner-berichte",
      tool_or_capability: "files.write",
      args_preview: "Ziel: Wochenbericht-KW40.md\nInhalt: 2,4 KB Markdown",
    },
    {
      id: "ap-2",
      caller: "workflow",
      integration_id: "ordner-berichte",
      tool_or_capability: "files.write",
      args_preview: "Ziel: Protokoll-2026-09-30.md\nInhalt: 5,1 KB Markdown",
    },
  ],
  audit: [
    {
      id: 12,
      ts: NOW + 50_000,
      caller: "agent_external",
      integration_id: "ordner-berichte",
      capability: "files.write",
      target: "Wochenbericht-KW40.md",
      outcome: "pending",
      detail_json: JSON.stringify({ phase: "approval", approval_id: "ap-1" }),
    },
    {
      id: 11,
      ts: NOW + 40_000,
      caller: "agent_external",
      integration_id: "ordner-archiv",
      capability: "files.read",
      target: null,
      outcome: "denied",
      detail_json: JSON.stringify({ reason: "grant_off", count: 3 }),
    },
    {
      id: 10,
      ts: NOW + 30_000,
      caller: "workflow",
      integration_id: "ordner-berichte",
      capability: "files.write",
      target: "Protokoll-2026-09-29.md",
      outcome: "ok",
      detail_json: JSON.stringify({ phase: "done" }),
    },
    {
      id: 9,
      ts: NOW + 20_000,
      caller: "workflow",
      integration_id: "ordner-berichte",
      capability: "files.write",
      target: "Bericht.md",
      outcome: "error",
      detail_json: JSON.stringify({
        phase: "done",
        error: "Zugriff verweigert",
      }),
    },
    {
      id: 8,
      ts: NOW + 10_000,
      caller: "user",
      integration_id: "ordner-berichte",
      capability: null,
      target: null,
      outcome: "ok",
      detail_json: JSON.stringify({ phase: "created", kind: "folder" }),
    },
  ],
};
