import type { Page } from "@playwright/test";

/**
 * Attrappe des Microsoft-365-Kontos (A5, #66) fuer `integrations-m365.spec.ts`.
 * Sie legt sich ueber `installTauriMock` (Kalender, Einstellungen) und
 * `installIntegrationsMock` (Register) und uebernimmt die `m365_*`-Kommandos
 * sowie die Zeilen der Art `m365` in `integrations_list`. Es wird nie ein Browser
 * geoeffnet und nie ein Server gefragt: `m365_sign_in` ist eine Attrappe, die
 * sofort gelingt, ein Fehlercode liefert oder wartet (`signIn: "pending"`), bis der
 * Test `window.__m365Finish()` ruft oder die Oberflaeche abbricht.
 *
 * Die Rechenregeln (Scopes je Faehigkeit, Zustand) spiegeln `m365::config` und
 * `m365::status`; die echten Regeln pruefen die Rust-Tests (`integrations::m365`).
 * Das Konto steht im localStorage (`lva.test.m365`): Neuladen behaelt es.
 */

export interface M365MockAccount {
  id: string;
  label: string;
  enabled?: boolean;
  client_id?: string;
  tenant?: string;
  caps?: string[];
  files_mode?: "full" | "app_folder";
  files_folder?: string;
  signed_in?: boolean;
  address?: string;
  /** Scopes, denen bei der letzten Anmeldung zugestimmt wurde. */
  granted?: string[];
  last_error?: string | null;
}

export interface M365MockOptions {
  accounts?: M365MockAccount[];
  /** `ok` (Standard), `pending` oder ein Fehlercode (`m365_...`). */
  signIn?: string;
  /** Fehlercode fuer `m365_test` (Standard: ok). */
  testCode?: string;
  /** Fehlercode fuer Testmail, Testdatei und Follow-up-Versand (Standard: ok). */
  actionCode?: string;
}

export const GUID = "11111111-2222-3333-4444-555555555555";

export const installM365Mock = async (
  page: Page,
  options: M365MockOptions = {},
) => {
  await page.addInitScript(
    ({ options, guid }) => {
      const w = window as any;
      const KEY = "lva.test.m365";
      const ORDER = ["mail.send", "files.write", "calendar.write"];
      const READS = new Set(["calendar.read", "files.read"]);
      const CALLERS = ["workflow", "agent_external", "agent_local"];
      const NOW = 1_790_000_000_000;
      const FILE = "local-voice-test-20261001-101500.txt";

      let state: any = null;
      try {
        const stored = window.localStorage.getItem(KEY);
        if (stored) state = JSON.parse(stored);
      } catch {
        /* ohne Speicher: frischer Zustand */
      }
      if (!state) {
        state = {
          nextId: 1,
          accounts: (options.accounts ?? []).map((a: any) => ({
            enabled: true,
            client_id: guid,
            tenant: "common",
            caps: ["mail.send", "files.write"],
            files_mode: "full",
            files_folder: "Local Voice AI",
            signed_in: false,
            address: "ich@example.com",
            granted: [],
            last_error: null,
            grants: {},
            ...a,
          })),
        };
      }
      w.__m365 = state;
      w.__m365Pending = null;
      const save = () => {
        try {
          window.localStorage.setItem(KEY, JSON.stringify(state));
        } catch {
          /* egal */
        }
      };
      const scopeOf = (cap: string, mode: string) =>
        cap === "mail.send"
          ? "Mail.Send"
          : cap === "files.write"
            ? mode === "app_folder"
              ? "Files.ReadWrite.AppFolder"
              : "Files.ReadWrite"
            : "Calendars.ReadWrite";
      const required = (a: any) => [
        "offline_access",
        "User.Read",
        ...ORDER.filter((c) => a.caps.includes(c)).map((c) =>
          scopeOf(c, a.files_mode),
        ),
      ];
      const missing = (a: any) =>
        required(a).filter((s) => !(a.granted ?? []).includes(s));
      const stateOf = (a: any) =>
        !a.client_id
          ? "not_configured"
          : a.caps.length === 0
            ? "no_capabilities"
            : !a.signed_in
              ? "needs_sign_in"
              : missing(a).length > 0
                ? "needs_consent"
                : "ready";
      const toStatus = (a: any) => ({
        state: stateOf(a),
        client_id: a.client_id,
        tenant: a.tenant,
        account: a.signed_in ? a.address : null,
        display_name: a.signed_in ? "Ich Selbst" : null,
        enabled_capabilities: ORDER.filter((c) => a.caps.includes(c)),
        available_capabilities: ORDER,
        files_mode: a.files_mode,
        files_folder: a.files_folder,
        required_scopes: required(a),
        granted_scopes: a.granted ?? [],
        missing_scopes: a.signed_in ? missing(a) : [],
        secret: a.signed_in ? "present" : "missing",
        signing_in: !!w.__m365Pending,
      });
      const toView = (a: any) => ({
        integration: {
          id: a.id,
          kind: "m365",
          label: a.label,
          enabled: a.enabled,
          direction: "both",
          config_json: JSON.stringify({
            client_id: a.client_id,
            tenant: a.tenant,
            enabled_capabilities: ORDER.filter((c) => a.caps.includes(c)),
            files_mode: a.files_mode,
            files_folder: a.files_folder,
          }),
          account_hint: "graph.microsoft.com",
          data_class: null,
          created_at: NOW,
          updated_at: NOW,
          last_ok_at: a.last_ok ?? null,
          last_error: a.last_error ?? null,
        },
        directions: ["read", "write", "both"],
        capabilities: [
          "calendar.read",
          "calendar.write",
          "mail.send",
          "files.read",
          "files.write",
        ].map((cap) => ({
          capability: cap,
          writes: !READS.has(cap),
          never_allow: false,
          direction_allows: true,
          modes: CALLERS.map((caller) => {
            const stored = a.grants?.[`${cap}|${caller}`] ?? null;
            const def =
              caller === "agent_external"
                ? "off"
                : READS.has(cap)
                  ? "allow"
                  : "ask";
            let effective = stored ?? def;
            let reason: string | null = null;
            if (!a.enabled) {
              effective = "off";
              reason = "integration_disabled";
            } else if (!a.caps.includes(cap)) {
              effective = "off";
              reason = "capability_not_enabled";
            } else if (effective === "off") {
              reason = "grant_off";
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
        secrets: [
          { slot: "token", status: a.signed_in ? "present" : "missing" },
        ],
        calendar_managed: false,
        pending_approvals: 0,
      });
      const find = (id: string) => {
        const a = state.accounts.find((x: any) => x.id === id);
        if (!a) throw `Integration nicht gefunden: ${id}`;
        return a;
      };
      const mine = (id: unknown) =>
        state.accounts.some((a: any) => a.id === id);
      const settle = (a: any) => {
        a.signed_in = true;
        a.granted = required(a);
        a.last_ok = NOW;
        a.last_error = null;
        save();
        return toStatus(a);
      };

      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        const log = () => w.__calls.push({ cmd, args });
        switch (cmd) {
          case "integrations_list": {
            const base = (await original(cmd, args)) ?? [];
            return [...base, ...state.accounts.map(toView)];
          }
          case "integration_update":
            if (mine(args.id)) {
              log();
              const a = find(args.id as string);
              if (args.label != null) a.label = args.label;
              if (args.enabled != null) a.enabled = args.enabled;
              save();
              return toView(a);
            }
            break;
          case "integration_delete":
            if (mine(args.id)) {
              log();
              state.accounts = state.accounts.filter(
                (a: any) => a.id !== args.id,
              );
              save();
              return null;
            }
            break;
          case "integration_set_grant":
            if (mine(args.id)) {
              log();
              const a = find(args.id as string);
              a.grants = a.grants ?? {};
              const key = `${args.capability}|${args.caller}`;
              if (args.mode == null) delete a.grants[key];
              else a.grants[key] = args.mode;
              save();
              return toView(a);
            }
            break;
          case "m365_create": {
            log();
            const label = String(args.label ?? "").trim();
            if (!label) throw "Der Name darf nicht leer sein.";
            const own = String(args.clientId ?? "").trim();
            if (own && !/^[0-9a-f-]{36}$/i.test(own))
              throw "m365_invalid|Das ist keine gültige Client-ID. Erwartet wird die Anwendungs-ID im Format 8-4-4-4-12.";
            const a = {
              id: `m365-${state.nextId++}`,
              label,
              enabled: true,
              client_id:
                own || (w.__settings?.calendar_graph_client_id as string) || "",
              tenant: String(args.tenant ?? "") || "common",
              caps: (args.capabilities as string[]) ?? [],
              files_mode: (args.filesMode as string) ?? "full",
              files_folder: String(args.filesFolder ?? "Local Voice AI"),
              signed_in: false,
              address: "ich@example.com",
              granted: [],
              last_error: null,
              grants: {},
            };
            state.accounts.push(a);
            save();
            return toView(a);
          }
          case "m365_status":
            log();
            return toStatus(find(args.id as string));
          case "m365_update_settings": {
            log();
            const a = find(args.id as string);
            const before = `${a.client_id}|${a.tenant}`;
            if (args.clientId != null) a.client_id = String(args.clientId);
            if (args.tenant != null) a.tenant = String(args.tenant) || "common";
            if (args.capabilities != null) a.caps = args.capabilities;
            if (args.filesMode != null) a.files_mode = args.filesMode;
            if (args.filesFolder != null) a.files_folder = args.filesFolder;
            if (`${a.client_id}|${a.tenant}` !== before) {
              a.signed_in = false;
              a.granted = [];
            }
            save();
            return toStatus(a);
          }
          case "m365_sign_in": {
            log();
            const a = find(args.id as string);
            const mode = options.signIn ?? "ok";
            if (mode === "pending") {
              return new Promise((resolve, reject) => {
                w.__m365Pending = {
                  finish: () => {
                    w.__m365Pending = null;
                    resolve(settle(a));
                  },
                  cancel: () => {
                    w.__m365Pending = null;
                    reject("m365_cancelled");
                  },
                };
              });
            }
            if (mode !== "ok") {
              a.last_error = "Anmeldung fehlgeschlagen";
              save();
              throw mode;
            }
            return settle(a);
          }
          case "m365_cancel_sign_in":
            log();
            if (w.__m365Pending) {
              w.__m365Pending.cancel();
              return true;
            }
            return false;
          case "m365_sign_out": {
            log();
            const a = find(args.id as string);
            a.signed_in = false;
            a.granted = [];
            save();
            return toStatus(a);
          }
          case "m365_test": {
            log();
            const a = find(args.id as string);
            if (!a.signed_in)
              return { ok: false, code: "m365_needs_sign_in", detail: null };
            if (options.testCode) {
              if (options.testCode === "m365_needs_sign_in") {
                a.signed_in = false;
                a.granted = [];
                save();
              }
              return { ok: false, code: options.testCode, detail: null };
            }
            a.last_ok = NOW;
            save();
            return { ok: true, code: "ok", detail: a.address };
          }
          case "m365_send_test_mail":
          case "m365_upload_test_file":
          case "meeting_followup_send_m365": {
            log();
            if (options.actionCode)
              return { ok: false, code: options.actionCode, detail: null };
            return {
              ok: true,
              code: "ok",
              detail: cmd === "m365_upload_test_file" ? FILE : null,
            };
          }
        }
        return original(cmd, args);
      };
      w.__m365Finish = () => w.__m365Pending?.finish();
    },
    { options, guid: GUID },
  );
};
