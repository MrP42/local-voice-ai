import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { NOW } from "./integrationsMock";

// Abzug der echten Kataloge (Rust: `workflows::ui::catalog_view`, `template_list`).
const fixture = JSON.parse(
  readFileSync(
    resolve(process.cwd(), "tests", "automationsCatalog.json"),
    "utf8",
  ),
) as { catalog: unknown; templates: unknown };

/**
 * Attrappe der Oberflaeche „Automationen“ (B7, #67) fuer `automations.spec.ts`. Sie legt sich
 * ueber `calendarMock.installTauriMock` und `integrationsMock.installIntegrationsMock` und
 * uebernimmt die Kommandos `workflow_*`.
 *
 * Katalog und Vorlagen sind ein Abzug der echten Rust-Tabellen (`workflows::ui`,
 * `tests/automationsCatalog.json`); die Pruefung und der Plan sind hier vereinfachte
 * Nachbildungen mit denselben Formen (JSON-Zeiger, Rechte-Ergebnis). Die echte Rechnung
 * pruefen die Rust-Tests (`cargo test --lib workflows::ui`).
 *
 * Die Freigabe `ap-2` (Integrationen-Attrappe) gehoert zu einem wartenden Lauf: wird sie im
 * Freigabedialog entschieden, geht der Lauf weiter (ja) oder endet abgelehnt (nein).
 */

export interface SeedWorkflow {
  id: string;
  enabled?: boolean;
  dry_run?: boolean;
  definition: Record<string, unknown>;
}

export interface SeedStep {
  step_id: string;
  action: string;
  state: string;
  attempt?: number;
  error?: string | null;
  approval_id?: string | null;
  output?: unknown;
}

export interface SeedRun {
  id: string;
  workflow_id: string;
  state: string;
  origin?: "trigger" | "manual" | "agent";
  dry_run?: boolean;
  error?: string | null;
  error_code?: string | null;
  steps: SeedStep[];
  provenance?: Array<Record<string, unknown>>;
  age_min?: number;
}

export interface AutomationsMockOptions {
  workflows?: SeedWorkflow[];
  runs?: SeedRun[];
  cloudOnly?: Array<{ workflow_id: string; name: string }>;
  channels?: Array<Record<string, unknown>>;
  /** Inhalt von Dateien fuer `workflow_read_file` (Pfad -> Text). */
  files?: Record<string, string>;
}

export const installAutomationsMock = async (
  page: Page,
  options: AutomationsMockOptions = {},
) => {
  await page.addInitScript(
    ({ options, now, fx }) => {
      const w = window as any;
      const catalog = fx.catalog;
      const templates = fx.templates;
      const state: any = {
        next: 1,
        nextRun: 1,
        workflows: [] as any[],
        runs: [] as any[],
      };
      w.__wf = state;
      w.__wfCalls = [];

      const titleOf = (id: string) =>
        catalog.actions.find((a: any) => a.id === id)?.title ?? id;
      const row = (wf: any) => ({
        id: wf.id,
        name: wf.def.name,
        enabled: wf.enabled,
        dry_run: wf.dry_run,
        updated_at: wf.updated_at,
        definition_json: JSON.stringify(wf.def),
      });
      const summary = (r: any) => ({
        id: r.id,
        workflow_id: r.workflow_id,
        workflow_name: r.workflow_name,
        origin: r.origin,
        state: r.state,
        dry_run: r.dry_run,
        created_at: r.created_at,
        started_at: r.started_at ?? null,
        ended_at: r.ended_at ?? null,
        error: r.error ?? null,
        error_code: r.error_code ?? null,
        wait_reason: null,
        cancel_requested: false,
      });
      const item = (wf: any) => {
        const runs = state.runs
          .filter((r: any) => r.workflow_id === wf.id)
          .sort((a: any, b: any) => b.created_at - a.created_at);
        return {
          id: wf.id,
          name: wf.def.name,
          enabled: wf.enabled,
          dry_run: wf.dry_run,
          updated_at: wf.updated_at,
          definition_json: JSON.stringify(wf.def),
          trigger_kind: wf.def.trigger?.type ?? "",
          step_count: (wf.def.steps ?? []).length,
          last_run: runs[0] ? summary(runs[0]) : null,
          open_runs: runs.filter((r: any) =>
            ["queued", "running", "awaiting_approval"].includes(r.state),
          ).length,
        };
      };
      void row;

      // --- Pruefung (vereinfachte Nachbildung) ---------------------------------
      const IDENT = /^[a-z][a-z0-9_]{0,31}$/;
      const validate = (text: string) => {
        let def: any;
        try {
          def = JSON.parse(text);
        } catch {
          return [
            { path: "", message: "Kein gültiges JSON (Zeile 1, Spalte 1)." },
          ];
        }
        const issues: Array<{ path: string; message: string }> = [];
        const add = (path: string, message: string) =>
          issues.push({ path, message });
        if (!def.name || !String(def.name).trim())
          add("/name", "Der Name darf nicht leer sein.");
        const trig = catalog.triggers.find(
          (t: any) => t.id === def.trigger?.type,
        );
        if (!trig) add("/trigger/type", "Unbekannter Auslöser.");
        else
          for (const f of trig.fields) {
            if (f.required && (def.trigger[f.name] ?? "") === "")
              add(`/trigger/${f.name}`, `Das Pflichtfeld „${f.name}“ fehlt.`);
          }
        const ids = new Set<string>();
        (def.steps ?? []).forEach((s: any, i: number) => {
          if (!IDENT.test(s.id ?? ""))
            add(`/steps/${i}/id`, "Die Kennung ist ungültig.");
          else if (ids.has(s.id))
            add(`/steps/${i}/id`, `Die Kennung „${s.id}“ kommt doppelt vor.`);
          ids.add(s.id);
          const a = catalog.actions.find((x: any) => x.id === s.action);
          if (!a) {
            add(`/steps/${i}/action`, `Unbekannter Baustein „${s.action}“.`);
            return;
          }
          for (const f of a.fields) {
            const v = s.params?.[f.name];
            if (f.required && (v === undefined || v === ""))
              add(
                `/steps/${i}/params/${f.name}`,
                `Das Pflichtfeld „${f.name}“ fehlt.`,
              );
            if (f.literal && typeof v === "string" && v.includes("{{"))
              add(
                `/steps/${i}/params/${f.name}`,
                "Dieses Feld darf kein {{…}} enthalten.",
              );
          }
          if (
            s.when &&
            (s.when.match(/\{\{/g) ?? []).length !==
              (s.when.match(/\}\}/g) ?? []).length
          )
            add(
              `/steps/${i}/when`,
              "Die Bedingung ist nicht lesbar: Klammern passen nicht.",
            );
        });
        return issues;
      };

      // --- Plan -------------------------------------------------------------------
      const integrations = () => (w.__reg?.integrations ?? []) as any[];
      const planOf = (text: string, workflowId: string | null) => {
        const issues = validate(text);
        if (issues.length > 0)
          return { schema: "lva-workflow-plan@1", valid: false, issues };
        const def = JSON.parse(text);
        const steps = (def.steps ?? []).map((s: any, i: number) => {
          const a = catalog.actions.find((x: any) => x.id === s.action);
          const via = a?.capability
            ? (s.params?.via ?? s.params?.target)
            : undefined;
          let permission: any = { required: false, result: "not_required" };
          if (a?.capability) {
            const found = integrations().find((x: any) => x.id === via);
            if (typeof via === "string" && via.includes("{{")) {
              permission = {
                required: true,
                result: "allowed",
                integration: via,
                capability: a.capability,
              };
            } else if (!found) {
              permission = {
                required: true,
                result: "denied",
                integration: via,
                capability: a.capability,
                message: "Die Integration gibt es nicht.",
              };
            } else if (
              a.capability === "mail.send" ||
              a.capability === "recording.start"
            ) {
              permission = {
                required: true,
                result: "needs_approval",
                integration: via,
                capability: a.capability,
                preview: `Ziel: ${s.params?.to ?? s.params?.title ?? ""}`,
              };
            } else {
              permission = {
                required: true,
                result: "allowed",
                integration: via,
                capability: a.capability,
              };
            }
          }
          const effect = a.effect_text.replace(
            /\{\{p\.(\w+)\}\}/g,
            (_m: string, k: string) => String(s.params?.[k] ?? ""),
          );
          return {
            index: i,
            id: s.id,
            action: s.action,
            title: a.title,
            label: s.label ?? null,
            status: "planned",
            condition: {
              expression: s.when ?? null,
              result: s.when ? "unknown" : "true",
            },
            params: s.params ?? {},
            unresolved: [],
            effect,
            effect_kind: a.effect,
            heavy: a.heavy_label
              ? { label: a.heavy_label, ram_mb: 3072 }
              : null,
            permission,
          };
        });
        const count = (r: string) =>
          steps.filter((s: any) => s.permission.result === r).length;
        const denied = count("denied");
        return {
          schema: "lva-workflow-plan@1",
          valid: true,
          dry_run: true,
          writes: "nothing",
          workflow: { id: workflowId, name: def.name },
          trigger: { type: def.trigger.type },
          trigger_sample: true,
          variables_without_default: Object.entries(def.variables ?? {})
            .filter(([, d]: any) => d.default === undefined)
            .map(([k]) => k),
          steps,
          summary: {
            steps: steps.length,
            planned: steps.length,
            skipped: 0,
            invalid: 0,
            allowed: count("allowed"),
            needs_approval: count("needs_approval"),
            denied,
            not_required: count("not_required"),
            heavy: steps.filter((s: any) => s.heavy).length,
            external_effects: steps.filter(
              (s: any) => s.effect_kind === "external",
            ).length,
            would_run_without_intervention: denied === 0,
          },
        };
      };

      // --- Seeds ---------------------------------------------------------------
      for (const s of options.workflows ?? []) {
        state.workflows.push({
          id: s.id,
          enabled: s.enabled ?? false,
          dry_run: s.dry_run ?? true,
          def: s.definition,
          updated_at: now - 3_600_000,
        });
      }
      for (const r of options.runs ?? []) {
        const wf = state.workflows.find((x: any) => x.id === r.workflow_id);
        const created = now - (r.age_min ?? 10) * 60_000;
        state.runs.push({
          id: r.id,
          workflow_id: r.workflow_id,
          workflow_name: wf?.def.name ?? r.workflow_id,
          origin: r.origin ?? "trigger",
          state: r.state,
          dry_run: r.dry_run ?? false,
          created_at: created,
          started_at: created + 1000,
          ended_at: ["done", "failed", "cancelled"].includes(r.state)
            ? created + 30_000
            : null,
          error: r.error ?? null,
          error_code: r.error_code ?? null,
          trigger: {
            name: "Eingang-Bericht.wav",
            path: "C:\\Ablage\\Berichte\\Eingang-Bericht.wav",
          },
          steps: r.steps.map((s: any, i: number) => ({
            run_id: r.id,
            step_id: s.step_id,
            attempt: s.attempt ?? 1,
            ordinal: i,
            action: s.action,
            action_title: titleOf(s.action),
            state: s.state,
            error_class: null,
            input_json: null,
            output_json: s.output ? JSON.stringify(s.output) : null,
            error: s.error ?? null,
            approval_id: s.approval_id ?? null,
            wake_at: null,
            started_at: created + 2000 * (i + 1),
            ended_at:
              s.state === "awaiting_approval" ? null : created + 2000 * (i + 2),
          })),
          provenance: r.provenance ?? [],
        });
      }

      const detail = (r: any) => {
        const failedUncertain = r.error_code === "effect_uncertain";
        return {
          run: summary(r),
          context_json: JSON.stringify({ trigger: r.trigger ?? {}, vars: {} }),
          definition_json: "{}",
          steps: r.steps,
          provenance: r.provenance,
          can_retry: r.state === "failed",
          retry_needs_confirmation: failedUncertain,
          can_cancel: ["queued", "running", "awaiting_approval"].includes(
            r.state,
          ),
        };
      };
      const findRun = (id: string) => {
        const r = state.runs.find((x: any) => x.id === id);
        if (!r) throw `Nicht gefunden: ${id}`;
        return r;
      };
      const findWf = (id: string) => {
        const wf = state.workflows.find((x: any) => x.id === id);
        if (!wf) throw `Nicht gefunden: ${id}`;
        return wf;
      };
      const record = (cmd: string, args: unknown) =>
        w.__wfCalls.push({ cmd, args });

      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        switch (cmd) {
          case "workflow_catalog":
            return catalog;
          case "workflow_templates":
            return templates;
          case "workflow_list":
            record(cmd, args);
            return state.workflows.map(item);
          case "workflow_status":
            return {
              cloud_only: (options.cloudOnly ?? []).map((c: any) => ({
                ...c,
                workflow_name:
                  state.workflows.find((x: any) => x.id === c.workflow_id)?.def
                    .name ?? c.workflow_id,
              })),
              channels: (options.channels ?? []).map((c: any) => ({
                workflow_name:
                  state.workflows.find((x: any) => x.id === c.workflow_id)?.def
                    .name ?? c.workflow_id,
                last_ok_ms: null,
                last_error: null,
                next_fetch_ms: now,
                ...c,
              })),
            };
          case "workflow_validate":
            record(cmd, args);
            return validate(String(args.definitionJson));
          case "workflow_save": {
            record(cmd, args);
            const text = String(args.definitionJson);
            const issues = validate(text);
            if (issues.length > 0) return { workflow: null, issues };
            const def = JSON.parse(text);
            if (args.id) {
              const wf = findWf(String(args.id));
              wf.def = def;
              wf.dry_run = true;
              wf.updated_at = now;
              return { workflow: item(wf), issues: [] };
            }
            const wf = {
              id: `wf-neu-${state.next++}`,
              enabled: false,
              dry_run: true,
              def,
              updated_at: now,
            };
            state.workflows.unshift(wf);
            return { workflow: item(wf), issues: [] };
          }
          case "workflow_import": {
            record(cmd, args);
            const text = String(args.definitionJson);
            const issues = validate(text);
            if (issues.length > 0) return { workflow: null, issues };
            const wf = {
              id: `wf-neu-${state.next++}`,
              enabled: false,
              dry_run: true,
              def: JSON.parse(text),
              updated_at: now,
            };
            state.workflows.unshift(wf);
            return { workflow: item(wf), issues: [] };
          }
          case "workflow_export": {
            record(cmd, args);
            const wf = findWf(String(args.id));
            return JSON.stringify(wf.def, null, 2) + "\n";
          }
          case "workflow_read_file": {
            record(cmd, args);
            const text = (options.files ?? {})[String(args.path)];
            if (text === undefined) throw "Die Datei lässt sich nicht öffnen.";
            return text;
          }
          case "workflow_export_file":
            record(cmd, args);
            return null;
          case "workflow_delete": {
            record(cmd, args);
            findWf(String(args.id));
            state.workflows = state.workflows.filter(
              (x: any) => x.id !== args.id,
            );
            return null;
          }
          case "workflow_set_enabled": {
            record(cmd, args);
            const wf = findWf(String(args.id));
            wf.enabled = !!args.enabled;
            return item(wf);
          }
          case "workflow_set_armed": {
            record(cmd, args);
            const wf = findWf(String(args.id));
            wf.dry_run = !args.armed;
            return item(wf);
          }
          case "workflow_plan":
            record(cmd, args);
            return JSON.stringify(
              planOf(
                String(args.definitionJson),
                (args.workflowId as string) ?? null,
              ),
            );
          case "workflow_run_start": {
            record(cmd, args);
            const wf = findWf(String(args.id));
            const dry = !!args.dryRun || wf.dry_run;
            if (!dry && !wf.enabled)
              throw `Der Ablauf „${wf.def.name}“ ist ausgeschaltet.`;
            const id = `run-neu-${state.nextRun++}`;
            state.runs.push({
              id,
              workflow_id: wf.id,
              workflow_name: wf.def.name,
              origin: "manual",
              state: "done",
              dry_run: dry,
              created_at: now + state.nextRun * 1000,
              started_at: now,
              ended_at: now + 500,
              error: null,
              error_code: null,
              steps: (wf.def.steps ?? []).map((s: any, i: number) => ({
                run_id: id,
                step_id: s.id,
                attempt: 1,
                ordinal: i,
                action: s.action,
                action_title: titleOf(s.action),
                state: dry ? "planned" : "done",
                error_class: null,
                input_json: null,
                output_json: null,
                error: null,
                approval_id: null,
                wake_at: null,
                started_at: now,
                ended_at: now,
              })),
              provenance: [],
            });
            return { run_id: id, created: true, dry_run: dry };
          }
          case "workflow_runs": {
            record(cmd, args);
            let list = state.runs.slice();
            if (args.workflowId)
              list = list.filter((r: any) => r.workflow_id === args.workflowId);
            if (args.openOnly)
              list = list.filter((r: any) =>
                ["queued", "running", "awaiting_approval"].includes(r.state),
              );
            return list
              .sort((a: any, b: any) => b.created_at - a.created_at)
              .map(summary);
          }
          case "workflow_run_detail": {
            record(cmd, args);
            return detail(findRun(String(args.runId)));
          }
          case "workflow_run_cancel": {
            record(cmd, args);
            const r = findRun(String(args.runId));
            if (!["queued", "running", "awaiting_approval"].includes(r.state))
              throw "Der Lauf ist schon beendet.";
            r.state = "cancelled";
            r.ended_at = now;
            return true;
          }
          case "workflow_run_retry": {
            record(cmd, args);
            const r = findRun(String(args.runId));
            if (r.state !== "failed")
              throw "Nur gescheiterte Läufe lassen sich wiederholen.";
            if (r.error_code === "effect_uncertain" && !args.acceptUncertain)
              throw "Es ist unklar, ob der Schritt schon Wirkung hatte: bitte ausdrücklich bestätigen.";
            // Die Attrappe beendet die Wiederholung sofort erfolgreich.
            const failed = r.steps.find((s: any) => s.state === "failed");
            if (failed) {
              r.steps.push({
                ...failed,
                attempt: failed.attempt + 1,
                state: "done",
                error: null,
              });
            }
            r.state = "done";
            r.error = null;
            r.error_code = null;
            return null;
          }
          case "workflow_approvals_changed":
            record(cmd, args);
            return null;
          case "approval_decide": {
            // Freigabe eines wartenden Laufs: der Lauf geht weiter oder endet abgelehnt.
            const r = state.runs.find((x: any) =>
              x.steps.some(
                (s: any) =>
                  s.approval_id === args.id && s.state === "awaiting_approval",
              ),
            );
            if (r) {
              const step = r.steps.find((s: any) => s.approval_id === args.id);
              if (args.approve) {
                step.state = "done";
                r.state = "done";
              } else {
                step.state = "denied";
                step.error = "Die Freigabe wurde verweigert.";
                r.state = "failed";
                r.error = "Die Freigabe wurde verweigert.";
                r.error_code = "denied";
              }
              r.ended_at = now;
            }
            return original(cmd, args);
          }
        }
        return original(cmd, args);
      };
    },
    { options, now: NOW, fx: fixture },
  );
};

export const wfCalls = (page: Page, cmd: string) =>
  page.evaluate(
    (c) =>
      ((window as any).__wfCalls as Array<{ cmd: string; args: any }>).filter(
        (x) => x.cmd === c,
      ),
    cmd,
  );

/** Zwei Ablaeufe und drei Laeufe fuer Screenshots und Pruefungen (zusammen mit `DEMO`). */
export const AUTOMATIONS_DEMO: AutomationsMockOptions = {
  workflows: [
    {
      id: "wf-eingang",
      enabled: true,
      dry_run: false,
      definition: {
        schema: "lva-workflow@1",
        name: "Eingangsordner: Protokoll als Word",
        description: "Neue Aufnahmen im Eingang werden protokolliert.",
        trigger: {
          type: "folder.file_added",
          integration: "ordner-berichte",
          extensions: ["wav", "mp3"],
        },
        variables: {
          protokoll_vorlage: { type: "string", default: "kunde" },
        },
        steps: [
          {
            id: "import",
            action: "meeting.import",
            label: "Datei importieren",
            params: {
              via: "{{trigger.integration}}",
              path: "{{trigger.path}}",
            },
          },
          {
            id: "protokoll",
            action: "meeting.minutes",
            params: { template: "{{vars.protokoll_vorlage}}" },
          },
          {
            id: "word",
            action: "export.document",
            params: {
              format: "docx",
              target: "ordner-berichte",
              name: "Protokoll",
            },
          },
        ],
      },
    },
    {
      id: "wf-kanal",
      enabled: false,
      dry_run: true,
      definition: {
        schema: "lva-workflow@1",
        name: "Kanal beobachten",
        trigger: {
          type: "youtube.channel_new_video",
          channel_id: "UC0123456789012345678901",
        },
        steps: [
          {
            id: "hinweis",
            action: "notify.local",
            params: { title: "Neues Video: {{trigger.title}}" },
          },
        ],
      },
    },
  ],
  runs: [
    {
      id: "run-ok",
      workflow_id: "wf-eingang",
      state: "done",
      age_min: 90,
      steps: [
        {
          step_id: "import",
          action: "meeting.import",
          state: "done",
          output: { meeting_id: "m-1" },
        },
        { step_id: "protokoll", action: "meeting.minutes", state: "done" },
        {
          step_id: "word",
          action: "export.document",
          state: "done",
          output: { path: "Protokoll.docx" },
        },
      ],
      provenance: [
        {
          id: "pv-1",
          subject_kind: "run_output",
          subject_id: "run-ok",
          subject_revision: null,
          created_at: NOW - 5_000_000,
          operation: "minutes",
          actor_kind: "workflow",
          actor_ref: "run-ok",
          provider: "llama.cpp",
          locality: "local",
          model_id: "gemma-4-e4b",
          model_label: "Gemma 4 E4B",
          usage_event_id: null,
          prompt_tokens: 4200,
          completion_tokens: 900,
          duration_ms: 42_000,
          sources: [{ kind: "transcript", id: "m-1" }],
          confidence: null,
          params_json: null,
        },
      ],
    },
    {
      id: "run-fail",
      workflow_id: "wf-eingang",
      state: "failed",
      age_min: 40,
      error: "Zugriff verweigert",
      error_code: "permanent",
      steps: [
        { step_id: "import", action: "meeting.import", state: "done" },
        { step_id: "protokoll", action: "meeting.minutes", state: "done" },
        {
          step_id: "word",
          action: "export.document",
          state: "failed",
          error: "Zugriff verweigert",
        },
      ],
    },
    {
      id: "run-wait",
      workflow_id: "wf-eingang",
      state: "awaiting_approval",
      age_min: 5,
      steps: [
        { step_id: "import", action: "meeting.import", state: "done" },
        { step_id: "protokoll", action: "meeting.minutes", state: "done" },
        {
          step_id: "word",
          action: "export.document",
          state: "awaiting_approval",
          approval_id: "ap-2",
        },
      ],
    },
    {
      id: "run-unsure",
      workflow_id: "wf-kanal",
      state: "failed",
      age_min: 20,
      error: "Es ist unklar, ob die Mitteilung schon erschien.",
      error_code: "effect_uncertain",
      steps: [
        { step_id: "hinweis", action: "notify.local", state: "uncertain" },
      ],
    },
  ],
  cloudOnly: [{ workflow_id: "wf-eingang", name: "Besprechung-Vertrieb.mp4" }],
  channels: [
    {
      workflow_id: "wf-kanal",
      channel_id: "UC0123456789012345678901",
      failures: 4,
      outage: true,
      last_error: "HTTP 404",
    },
  ],
};
