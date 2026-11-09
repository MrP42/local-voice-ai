import type { Page } from "@playwright/test";

/**
 * Attrappe des Sprachdienstes (ADR-0021, S2) für `sprachdienst.spec.ts`. Sie legt sich über
 * `installTauriMock` und übernimmt die `sprachdienst_*`-Kommandos. Es gibt weder Pipe noch
 * Socket; die echten Regeln (Handschlag, Frist, Ratenbremse, Rechte) prüfen die Rust-Tests
 * (`cargo test --lib sprachdienst::`).
 */

export interface MockSprachZugang {
  zugang: string;
  anzeige: string;
  client?: string;
  hoeren?: boolean;
  sprechen?: boolean;
  erstellt_ms?: number;
  zuletzt_ms?: number | null;
  widerrufen_ms?: number | null;
}

export interface MockKopplung {
  id: string;
  client?: string;
  anzeige: string;
  code: string;
  /** Millisekunden bis zum Verfall, vom festgesetzten Zeitpunkt aus. */
  rest_ms?: number;
}

export interface SprachdienstMockOptions {
  aktiv?: boolean;
  zugaenge?: MockSprachZugang[];
  kopplungen?: MockKopplung[];
  fehler?: string | null;
}

export const ENDPUNKT = "/run/user/1000/wai/wai-sprache.sock";

export const installSprachdienstMock = async (
  page: Page,
  now: number,
  options: SprachdienstMockOptions = {},
) => {
  await page.addInitScript(
    ({ options, now, endpunkt }) => {
      const w = window as any;
      const state = {
        aktiv: options.aktiv ?? false,
        fehler: options.fehler ?? null,
        zugaenge: (options.zugaenge ?? []).map((z: any) => ({
          client: "corporate-llm",
          hoeren: false,
          sprechen: false,
          erstellt_ms: now - 86_400_000,
          zuletzt_ms: null,
          widerrufen_ms: null,
          ...z,
        })),
        kopplungen: (options.kopplungen ?? []).map((k: any) => ({
          id: k.id,
          client: k.client ?? "corporate-llm",
          anzeige: k.anzeige,
          code: k.code,
          bis_ms: now + (k.rest_ms ?? 100_000),
        })),
      };
      w.__sprache = state;
      const status = () => ({
        aktiv: state.aktiv,
        laeuft: state.aktiv && !state.fehler,
        endpunkt: state.aktiv && !state.fehler ? endpunkt : null,
        fehler: state.aktiv ? state.fehler : null,
      });
      const find = (id: string) => {
        const z = state.zugaenge.find((x: any) => x.zugang === id);
        if (!z) throw "Diese App ist nicht gekoppelt.";
        return z;
      };
      const original = w.__TAURI_INTERNALS__.invoke;
      w.__TAURI_INTERNALS__.invoke = async (
        cmd: string,
        args: Record<string, unknown> = {},
      ) => {
        switch (cmd) {
          case "sprachdienst_status":
            w.__calls.push({ cmd, args });
            return status();
          case "sprachdienst_aktiv_setzen":
            w.__calls.push({ cmd, args });
            state.aktiv = Boolean(args.aktiv);
            return status();
          case "sprachdienst_zugaenge":
            w.__calls.push({ cmd, args });
            return state.zugaenge.map((z: any) => ({ ...z }));
          case "sprachdienst_recht_setzen": {
            w.__calls.push({ cmd, args });
            const z = find(args.zugang as string);
            if (z.widerrufen_ms) throw "Diese App ist nicht gekoppelt.";
            z[args.recht as string] = Boolean(args.an);
            return { ...z };
          }
          case "sprachdienst_widerrufen": {
            w.__calls.push({ cmd, args });
            const z = find(args.zugang as string);
            z.hoeren = false;
            z.sprechen = false;
            z.widerrufen_ms = now;
            return { ...z };
          }
          case "sprachdienst_entfernen":
            w.__calls.push({ cmd, args });
            find(args.zugang as string);
            state.zugaenge = state.zugaenge.filter(
              (z: any) => z.zugang !== args.zugang,
            );
            return null;
          case "sprachdienst_kopplungen":
            w.__calls.push({ cmd, args });
            return state.kopplungen.map((k: any) => ({ ...k }));
          case "sprachdienst_kopplung_entscheiden": {
            w.__calls.push({ cmd, args });
            const k = state.kopplungen.find((x: any) => x.id === args.id);
            if (!k) return false;
            state.kopplungen = state.kopplungen.filter(
              (x: any) => x.id !== args.id,
            );
            if (args.bestaetigen) {
              state.zugaenge.push({
                zugang: `z-${k.id}`,
                client: k.client,
                anzeige: k.anzeige,
                hoeren: false,
                sprechen: false,
                erstellt_ms: now,
                zuletzt_ms: null,
                widerrufen_ms: null,
              });
            }
            return true;
          }
          default:
            return original(cmd, args);
        }
      };
    },
    { options, now, endpunkt: ENDPUNKT },
  );
};
