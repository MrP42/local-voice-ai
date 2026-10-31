import { useCallback, useEffect, useState } from "react";
import { commands, type TtsProviderEntry } from "@/bindings";
import { useSettings } from "../../../../hooks/useSettings";
import { asTtsError, nextApprovalStep } from "./providerModel";

export type ProviderDialogState = {
  kind: "license" | "remote";
  id: string;
} | null;

/**
 * Zustand und Aktionen der Sprachausgabe-Anbieter: Liste laden, pruefen,
 * aktivieren, Freigabe-Ablauf (Lizenz, Fernzugriff). Vorher stand das in der
 * Gruppe unter Einstellungen > Ausgabe; die Verwaltung unter Modelle > Vorlesen
 * nutzt dieselbe Logik. Die Oberflaeche liest nur, was die Befehle liefern --
 * neue Anbieter erscheinen ohne Aenderung hier.
 */
export function useTtsProviders() {
  const [entries, setEntries] = useState<TtsProviderEntry[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [dialog, setDialog] = useState<ProviderDialogState>(null);
  const { refreshSettings } = useSettings();

  const reload = useCallback(async (): Promise<TtsProviderEntry[] | null> => {
    setChecking(true);
    try {
      const result = await commands.ttsProvidersList();
      if (result.status === "ok") {
        setEntries(result.data);
        setLoadError(null);
        return result.data;
      }
      setLoadError(result.error.message);
    } catch (e) {
      setLoadError(asTtsError(e).message);
    } finally {
      setChecking(false);
    }
    return null;
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  /** Nach einem Schritt der Freigabe: der naechste Dialog, falls noch einer faellig ist. */
  const advance = useCallback((id: string, list: TtsProviderEntry[] | null) => {
    const entry = list?.find((e) => e.descriptor.id === id);
    if (!entry) return;
    const step = nextApprovalStep(entry);
    if (step) setDialog({ kind: step, id });
  }, []);

  const run = useCallback(
    async (
      id: string,
      action: () => Promise<
        { status: "ok" } | { status: "error"; error: unknown }
      >,
    ): Promise<boolean> => {
      setBusy(id);
      setActionError(null);
      try {
        const result = await action();
        if (result.status === "error") {
          setActionError(asTtsError(result.error).message);
          return false;
        }
        return true;
      } catch (e) {
        setActionError(asTtsError(e).message);
        return false;
      } finally {
        setBusy(null);
      }
    },
    [],
  );

  const enable = useCallback(
    async (id: string, enabled: boolean) => {
      const ok = await run(id, () =>
        commands.ttsProviderSetEnabled(id, enabled),
      );
      const list = await reload();
      if (ok && enabled) advance(id, list);
    },
    [run, reload, advance],
  );

  const recheck = useCallback(async (id: string) => {
    setBusy(id);
    try {
      const result = await commands.ttsProviderHealth(id);
      if (result.status === "ok") {
        setEntries((cur) =>
          (cur ?? []).map((e) =>
            e.descriptor.id === id ? { ...e, health: result.data } : e,
          ),
        );
      } else setActionError(result.error.message);
    } catch (e) {
      setActionError(asTtsError(e).message);
    } finally {
      setBusy(null);
    }
  }, []);

  const revokeRemote = useCallback(
    async (id: string) => {
      await run(id, () => commands.ttsProviderSetAllowRemote(id, false));
      await reload();
    },
    [run, reload],
  );

  /** Eine Aenderung in den Einstellungen eines Anbieters: Liste und Vorlesen-Dropdown neu lesen. */
  const changed = useCallback(() => {
    void reload();
    void refreshSettings();
  }, [reload, refreshSettings]);

  return {
    entries,
    loadError,
    actionError,
    busy,
    checking,
    dialog,
    setDialog,
    reload,
    advance,
    enable,
    recheck,
    revokeRemote,
    changed,
  };
}

export type TtsProvidersState = ReturnType<typeof useTtsProviders>;
