import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  events,
  type PreviewPhase,
  type TtsErrorKind,
  type TtsProviderPreview,
} from "@/bindings";
import { asTtsError, previewIsActive } from "./providerModel";

export interface PreviewState {
  providerId: string;
  /** `null`, solange die Antwort des Befehls noch aussteht. */
  requestId: string | null;
  phase: PreviewPhase | "starting";
  ttfaMs: number | null;
  errorKind: TtsErrorKind | null;
  message: string | null;
}

function merge(state: PreviewState, event: TtsProviderPreview): PreviewState {
  // Ein beendeter Lauf bleibt beendet (verspaetete Ereignisse zaehlen nicht).
  if (!previewIsActive(state.phase)) return state;
  return {
    ...state,
    phase: event.phase,
    ttfaMs: event.ttfa_ms ?? state.ttfaMs,
    errorKind: event.error_kind ?? state.errorKind,
    message: event.message ?? state.message,
  };
}

/**
 * Probestimme: Start, Ereignisse, Abbruch. Das Ereignis `started` kann VOR der
 * Antwort des Befehls eintreffen -- dann kennt die Oberflaeche die
 * `request_id` noch nicht. Solange ein Start aussteht, werden unbekannte
 * Ereignisse gepuffert und nach der Antwort ueber die `request_id` zugeordnet.
 */
export function useProviderPreview() {
  const [state, setState] = useState<PreviewState | null>(null);
  const activeRef = useRef<string | null>(null);
  const startingRef = useRef(false);
  const bufferRef = useRef<TtsProviderPreview[]>([]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void events.ttsProviderPreview
      .listen((event) => {
        const e = event.payload;
        if (e.request_id === activeRef.current) {
          setState((prev) => (prev ? merge(prev, e) : prev));
        } else if (startingRef.current) {
          bufferRef.current.push(e);
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  // Seite verlassen: eine laufende Probestimme nicht weiterspielen lassen.
  useEffect(
    () => () => {
      const id = activeRef.current;
      if (id) void commands.ttsProviderCancel(id).catch(() => undefined);
    },
    [],
  );

  const start = useCallback(
    async (providerId: string, text: string, voice: string | null) => {
      startingRef.current = true;
      bufferRef.current = [];
      activeRef.current = null;
      setState({
        providerId,
        requestId: null,
        phase: "starting",
        ttfaMs: null,
        errorKind: null,
        message: null,
      });
      try {
        const result = await commands.ttsProviderPreview(
          providerId,
          text,
          voice,
        );
        if (result.status !== "ok") {
          const err = asTtsError(result.error);
          setState({
            providerId,
            requestId: null,
            phase: "error",
            ttfaMs: null,
            errorKind: err.kind,
            message: err.message,
          });
          return;
        }
        const requestId = result.data;
        activeRef.current = requestId;
        const buffered = bufferRef.current.filter(
          (e) => e.request_id === requestId,
        );
        setState((prev) => {
          let next: PreviewState = {
            ...(prev ?? {
              providerId,
              ttfaMs: null,
              errorKind: null,
              message: null,
            }),
            providerId,
            requestId,
            phase: "starting",
          } as PreviewState;
          for (const e of buffered) next = merge(next, e);
          return next;
        });
      } catch (error) {
        const err = asTtsError(error);
        setState({
          providerId,
          requestId: null,
          phase: "error",
          ttfaMs: null,
          errorKind: err.kind,
          message: err.message,
        });
      } finally {
        startingRef.current = false;
        bufferRef.current = [];
      }
    },
    [],
  );

  const cancel = useCallback(async () => {
    const id = activeRef.current;
    if (!id) return;
    const result = await commands.ttsProviderCancel(id).catch(() => null);
    // `false`: lief nicht mehr. Kommt dazu kein Ereignis, bliebe der Knopf stehen.
    if (result && result.status === "ok" && !result.data) {
      setState((prev) =>
        prev && prev.requestId === id && previewIsActive(prev.phase)
          ? { ...prev, phase: "cancelled" }
          : prev,
      );
    }
  }, []);

  return { state, start, cancel };
}
