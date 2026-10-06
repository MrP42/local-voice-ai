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
  /** Weitere Konfiguration (A6: Server, Vault, Endpunkt ...), ohne Geheimnis. */
  config?: Record<string, unknown>;
  /** Ein Geheimnis (Passwort, Schluessel) ist hinterlegt. */
  secret?: boolean;
}

export interface IntegrationsMockOptions {
  integrations?: MockIntegration[];
  approvals?: MockApproval[];
  audit?: MockAudit[];
  /** Ergebnis von `integration_test` (Standard: ok). */
  testCode?: string;
  /** Pfad, den der Ordnerdialog liefert. */
  pickedPath?: string;
  /** Ergebnis von `integration_test` je Art (A6); Standard je Art ok. */
  tests?: Record<string, { code: string; detail?: string }>;
  /** Fehlertext von `integration_send_test_mail` (Standard: Erfolg). */
  mailError?: string;
  /** Treffer von `wissen_suchen` (Standard: zwei Beispiele). */
  wissenHits?: Array<Record<string, unknown>>;
  /** Fehlertext von `wissen_suchen`. */
  wissenError?: string;
  /** Fehlertext von „ablegen in“ (Ordner und Vault). */
  placeError?: string;
  /** Ergebnis von „Notiz in den Vault“ (Standard: created). */
  vaultResult?: "created" | "updated" | "unchanged";
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
        obsidian: {
          directions: ["read", "write", "both"],
          caps: ["vault.write", "files.read"],
        },
        wissen: {
          directions: ["read"],
          caps: ["knowledge.search", "knowledge.read"],
        },
        webhook: { directions: ["write"], caps: ["webhook.post"] },
        // Dienst: die Faehigkeiten haengen am Dienst (`SERVICES`), siehe `toView`.
        service: { directions: ["write"], caps: [] },
        youtube: { directions: ["read"], caps: ["media.fetch", "youtube.add"] },
        agent: {
          directions: ["read", "write", "both"],
          caps: [
            "meeting.create",
            "recording.start",
            "transcribe.file",
            "tts.render",
            "youtube.add",
            "workflow.read",
            "workflow.run",
          ],
        },
      };
      // Wie `services::config::catalog` (Auszug: Felder, Anmeldung, Hosts der Webhooks).
      const SERVICES: Record<
        string,
        {
          label: string;
          auth: string;
          capabilities: string[];
          fields: Array<{ key: string; required: boolean }>;
          hosts?: string[];
        }
      > = {
        slack: {
          label: "Slack",
          auth: "webhook_url",
          capabilities: ["chat.post"],
          fields: [],
          hosts: ["hooks.slack.com"],
        },
        teams: {
          label: "Microsoft Teams",
          auth: "webhook_url",
          capabilities: ["chat.post"],
          fields: [],
          hosts: [".logic.azure.com", ".powerplatform.com"],
        },
        discord: {
          label: "Discord",
          auth: "webhook_url",
          capabilities: ["chat.post"],
          fields: [],
          hosts: ["discord.com", "discordapp.com"],
        },
        notion: {
          label: "Notion",
          auth: "bearer",
          capabilities: ["page.write"],
          fields: [{ key: "parent_page_id", required: true }],
        },
        confluence: {
          label: "Confluence",
          auth: "basic_email_token",
          capabilities: ["page.write"],
          fields: [
            { key: "site", required: true },
            { key: "email", required: true },
            { key: "space_id", required: true },
            { key: "parent_page_id", required: false },
          ],
        },
        asana: {
          label: "Asana",
          auth: "bearer",
          capabilities: ["task.create"],
          fields: [{ key: "project_id", required: true }],
        },
        clickup: {
          label: "ClickUp",
          auth: "raw_authorization",
          capabilities: ["task.create"],
          fields: [{ key: "list_id", required: true }],
        },
        jira: {
          label: "Jira",
          auth: "basic_email_token",
          capabilities: ["task.create"],
          fields: [
            { key: "site", required: true },
            { key: "email", required: true },
            { key: "project_key", required: true },
            { key: "issue_type", required: false },
          ],
        },
        trello: {
          label: "Trello",
          auth: "trello_key_token",
          capabilities: ["task.create"],
          fields: [
            { key: "api_key", required: true },
            { key: "list_id", required: true },
          ],
        },
        todoist: {
          label: "Todoist",
          auth: "bearer",
          capabilities: ["task.create"],
          fields: [{ key: "project_id", required: false }],
        },
        monday: {
          label: "monday.com",
          auth: "raw_authorization",
          capabilities: ["task.create"],
          fields: [
            { key: "board_id", required: true },
            { key: "group_id", required: false },
          ],
        },
        linear: {
          label: "Linear",
          auth: "raw_authorization",
          capabilities: ["task.create"],
          fields: [{ key: "team_id", required: true }],
        },
        github: {
          label: "GitHub",
          auth: "bearer",
          capabilities: ["task.create"],
          fields: [{ key: "repo", required: true }],
        },
        hubspot: {
          label: "HubSpot",
          auth: "bearer",
          capabilities: ["crm.write", "task.create"],
          fields: [],
        },
        pipedrive: {
          label: "Pipedrive",
          auth: "api_token_header",
          capabilities: ["crm.write", "task.create"],
          fields: [],
        },
        airtable: {
          label: "Airtable",
          auth: "bearer",
          capabilities: ["record.write"],
          fields: [
            { key: "base_id", required: true },
            { key: "table", required: true },
          ],
        },
      };
      const READS = new Set([
        "calendar.read",
        "files.read",
        "knowledge.search",
        "knowledge.read",
        "media.fetch",
        "workflow.read",
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
            direction:
              KINDS[i.kind]?.directions.length === 1
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
        config_json: JSON.stringify({
          ...(i.path ? { path: i.path } : {}),
          ...(i.config ?? {}),
        }),
        account_hint: (i.config?.host as string) ?? null,
        data_class: null,
        created_at: now,
        updated_at: now,
        last_ok_at: null,
        last_error: i.last_error,
      });
      const capsOf = (i: any): string[] =>
        i.kind === "service"
          ? (SERVICES[i.config?.service]?.capabilities ?? [])
          : (KINDS[i.kind] ?? KINDS.folder).caps;
      const toView = (i: any) => {
        const kind = KINDS[i.kind] ?? KINDS.folder;
        return {
          integration: toIntegration(i),
          directions: kind.directions,
          capabilities: capsOf(i).map((cap) => ({
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
          secrets:
            i.kind === "smtp" ||
            i.kind === "wissen" ||
            i.kind === "webhook" ||
            i.kind === "service"
              ? [
                  {
                    slot:
                      i.kind === "smtp"
                        ? "password"
                        : i.kind === "webhook"
                          ? "url"
                          : "token",
                    status: i.secret ? "present" : "missing",
                  },
                ]
              : [],
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
      // Wie `webhook::parse_url`: nur https, http nur gegen diesen Rechner; liefert den Server.
      const webhookHost = (raw: unknown): string => {
        const text = String(raw ?? "").trim();
        if (!text) throw "Die Adresse des Webhooks fehlt.";
        let url: URL;
        try {
          url = new URL(text);
        } catch {
          throw "Die Adresse des Webhooks ist ungültig (zum Beispiel https://host/webhook/abc).";
        }
        const loopback = ["localhost", "127.0.0.1", "[::1]"].includes(
          url.hostname,
        );
        if (url.protocol === "http:" && !loopback)
          throw "Der Webhook muss https verwenden (http nur auf diesem Rechner).";
        if (url.protocol !== "https:" && url.protocol !== "http:")
          throw "Der Webhook muss mit https:// beginnen.";
        return url.host;
      };
      // Wie `services::config::check_secret`: Webhook-Adressen nur auf den Hosts des Dienstes.
      const serviceHost = (service: string, raw: unknown): string => {
        let url: URL;
        try {
          url = new URL(String(raw ?? "").trim());
        } catch {
          throw "Die Webhook-Adresse ist ungültig (vollständig mit https:// einfügen).";
        }
        if (url.protocol !== "https:") throw "Der Dienst muss https verwenden.";
        const ok = (SERVICES[service].hosts ?? []).some((h) =>
          h.startsWith(".")
            ? url.hostname.endsWith(h)
            : url.hostname === h,
        );
        if (!ok)
          throw `Die Adresse gehört nicht zu diesem Dienst (${url.hostname}).`;
        return url.hostname;
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
            if (
              args.capability === "recording.start" &&
              args.mode === "allow"
            ) {
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
            const i = find(args.id as string);
            const per = options.tests?.[i.kind];
            if (per) {
              return {
                ok: per.code.endsWith("_ok"),
                code: per.code,
                detail: per.detail ?? null,
              };
            }
            if (i.kind === "service") {
              return SERVICES[i.config?.service]?.auth === "webhook_url"
                ? {
                    ok: true,
                    code: "service_ok",
                    detail: "Adresse geprüft (ohne Testnachricht).",
                  }
                : { ok: true, code: "service_ok", detail: "Verbunden (Kundenprojekt)." };
            }
            const okCode: Record<string, string> = {
              smtp: "smtp_ok",
              obsidian: "vault_ok",
              wissen: "wissen_ok",
            };
            const code = options.testCode ?? okCode[i.kind] ?? "folder_ok";
            return { ok: code.endsWith("_ok"), code, detail: null };
          }
          case "integrations_audit_list": {
            w.__calls.push({ cmd, args });
            let rows = state.audit as any[];
            if (args.integrationId)
              rows = rows.filter(
                (r) => r.integration_id === args.integrationId,
              );
            if (args.outcome)
              rows = rows.filter((r) => r.outcome === args.outcome);
            if (args.caller)
              rows = rows.filter((r) => r.caller === args.caller);
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
            // Wie das Backend (B7n): eine Freigabe zum Aufnehmen gilt nur mit der Bestaetigung
            // der Einwilligung; das Beenden einer Aufnahme und das Ablehnen brauchen sie nicht.
            if (
              args.approve &&
              !args.consentConfirmed &&
              a.tool_or_capability === "recording.start" &&
              !String(a.args_preview ?? "").startsWith("Ziel: stop_recording")
            )
              throw "consent_required";
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
          case "integration_services":
            return Object.entries(SERVICES).map(([id, s]) => ({
              id,
              label: s.label,
              auth: s.auth,
              capabilities: s.capabilities,
              fields: s.fields,
              token_help_url: `https://hilfe.example.org/${id}`,
            }));
          case "integration_create_with_settings": {
            w.__calls.push({ cmd, args });
            const st = (args.settings ?? {}) as Record<string, any>;
            const kind = String(args.kind);
            const label = String(args.label ?? "").trim();
            if (!label) throw "Der Name darf nicht leer sein.";
            const cfg: Record<string, unknown> = {};
            let secret = false;
            if (kind === "smtp") {
              if (!String(st.host ?? "").trim()) throw "Der Server fehlt.";
              if (st.security === "plain")
                throw "Eine unverschlüsselte Verbindung ist nur zu diesem Rechner erlaubt. Bitte STARTTLS oder TLS wählen.";
              if (String(st.username ?? "") && !st.secret)
                throw "Das Passwort fehlt. Bitte in der Integration neu eintragen.";
              Object.assign(cfg, {
                host: st.host,
                port: st.port || 587,
                security: st.security ?? "starttls",
                username: st.username ?? "",
                from_address: st.from_address ?? "",
                from_name: st.from_name ?? "",
              });
              secret = !!st.secret;
            } else if (kind === "obsidian") {
              const path = String(st.path ?? "").trim();
              if (!path) throw "vault_path_missing";
              if (!/^[A-Za-z]:[\\/]/.test(path)) throw "vault_path_relative";
              if (path.includes("gibt-es-nicht")) throw "vault_path_not_found";
              Object.assign(cfg, {
                path,
                subfolder: st.subfolder || "00_inbox",
                context_area: st.context_area || "beruf",
                tier: st.tier || "propose",
              });
            } else if (kind === "wissen") {
              const endpoint = String(st.endpoint ?? "").trim();
              if (!endpoint) throw "Die Adresse des Endpunkts fehlt.";
              if (
                !/^https:\/\//.test(endpoint) &&
                !/^http:\/\/(127\.0\.0\.1|localhost)/.test(endpoint)
              ) {
                throw "Der Endpunkt muss https verwenden (http nur auf diesem Rechner).";
              }
              if (!st.secret)
                throw "Der Zugangsschlüssel fehlt. Bitte in der Integration neu eintragen.";
              Object.assign(cfg, {
                endpoint,
                search_tool: st.search_tool || "wissen_suchen",
                area: st.area ?? "",
              });
              secret = true;
            } else if (kind === "webhook") {
              // Wie das Backend: die Adresse ist das Geheimnis, in der Konfiguration
              // steht nur der Server.
              Object.assign(cfg, { host: webhookHost(st.secret) });
              secret = true;
            } else if (kind === "service") {
              const svc = SERVICES[String(st.service)];
              if (!svc) throw "service_unknown";
              const fields = (st.fields ?? {}) as Record<string, string>;
              for (const f of svc.fields) {
                if (f.required && !String(fields[f.key] ?? "").trim())
                  throw `Für ${svc.label} fehlt das Feld „${f.key}“.`;
              }
              if (!st.secret)
                throw "Der Schlüssel bzw. die Webhook-Adresse fehlt.";
              cfg.service = st.service;
              for (const f of svc.fields) {
                const v = String(fields[f.key] ?? "").trim();
                if (v) cfg[f.key] = v;
              }
              if (svc.auth === "webhook_url") {
                cfg.host = serviceHost(String(st.service), st.secret);
              } else if (/\s/.test(String(st.secret).trim())) {
                throw "Der Schlüssel enthält Leer- oder Steuerzeichen. Bitte nur den Schlüssel einfügen.";
              }
              secret = true;
            } else {
              throw "kind_not_available";
            }
            const i = {
              id: `int-${state.nextId++}`,
              kind,
              label,
              enabled: true,
              direction:
                args.direction ??
                KINDS[kind].directions[
                  KINDS[kind].directions.length === 1 ? 0 : 2
                ],
              path: null,
              last_error: null,
              grants: {},
              config: cfg,
              secret,
            };
            state.integrations.push(i);
            audit(i.id, null, { phase: "created", kind });
            save();
            return toView(i);
          }
          case "integration_update_settings": {
            w.__calls.push({ cmd, args });
            const i = find(args.id as string);
            const st = (args.settings ?? {}) as Record<string, any>;
            const map: Record<string, string> = {
              host: "host",
              port: "port",
              security: "security",
              username: "username",
              from_address: "from_address",
              from_name: "from_name",
              subfolder: "subfolder",
              context_area: "context_area",
              tier: "tier",
              endpoint: "endpoint",
              search_tool: "search_tool",
              area: "area",
            };
            i.config = i.config ?? {};
            if (i.kind === "obsidian" || i.kind === "folder") {
              const path = String(st.path ?? "").trim();
              if (path && !/^[A-Za-z]:[\\/]/.test(path)) {
                throw i.kind === "obsidian"
                  ? "vault_path_relative"
                  : "folder_path_relative";
              }
              if (path) i.path = path;
            }
            for (const [k, v] of Object.entries(map)) {
              if (st[k] != null) i.config[v] = st[k];
            }
            if (i.kind === "webhook" && st.secret) {
              i.config.host = webhookHost(st.secret);
            }
            if (i.kind === "service") {
              for (const [k, v] of Object.entries(
                (st.fields ?? {}) as Record<string, string>,
              )) {
                i.config[k] = v;
              }
              if (st.secret && SERVICES[i.config.service]?.auth === "webhook_url")
                i.config.host = serviceHost(i.config.service, st.secret);
            }
            if (st.secret) i.secret = true;
            audit(i.id, null, {
              phase: "updated",
              changes: { settings: true, secret: !!st.secret },
            });
            save();
            return toView(i);
          }
          case "integration_send_test_mail": {
            w.__calls.push({ cmd, args });
            find(args.id as string);
            if (options.mailError) throw options.mailError;
            audit(args.id as string, "mail.send", { phase: "done" });
            save();
            return null;
          }
          case "wissen_suchen": {
            w.__calls.push({ cmd, args });
            find(args.id as string);
            if (options.wissenError) throw options.wissenError;
            return (
              options.wissenHits ?? [
                {
                  title: "Preisliste 2026",
                  path: "10_contexts/wai/preise.md",
                  area: "wai",
                  snippet: "Der Tagessatz beträgt 1.200 EUR.",
                  score: 0.91,
                  source: "vault",
                  page: null,
                  document_id: "d1",
                },
                {
                  title: "Handbuch",
                  path: "buch:handbuch",
                  area: "wai",
                  snippet: "Kapitel 3",
                  score: 0.5,
                  source: "buch",
                  page: 42,
                  document_id: "d2",
                },
              ]
            );
          }
          case "integration_export_to_folder": {
            w.__calls.push({ cmd, args });
            const i = find(args.id as string);
            if (options.placeError) throw options.placeError;
            audit(i.id, "files.write", { phase: "done" });
            save();
            return { rel: `Besprechung.${args.format}`, bytes: 1234 };
          }
          case "integration_save_to_vault": {
            w.__calls.push({ cmd, args });
            const i = find(args.id as string);
            if (options.placeError) throw options.placeError;
            audit(i.id, "vault.write", { phase: "done" });
            save();
            return {
              rel: "00_inbox/2026-10-01 Besprechung.md",
              result: options.vaultResult ?? "created",
            };
          }
          case "plugin:dialog|open":
            return options.pickedPath ?? "C:\\Ablage\\Berichte";
          // MCP (wie in meeting-mcp.spec.ts)
          case "meeting_mcp_info":
            return {
              exe_path: "C:\\Program Files\\Local Voice AI\\local-voice-ai.exe",
            };
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
