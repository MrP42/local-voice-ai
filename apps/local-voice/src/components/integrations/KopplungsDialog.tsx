import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type SprachKopplung } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { errorText } from "./model";

/** „123456“ als „123 456“, damit der Vergleich mit der anderen App leichter fällt. */
const gruppiert = (code: string): string =>
  code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;

/**
 * Kopplungsanfrage einer App an den Sprachdienst (ADR-0021). Der Dialog steht global in der App:
 * eine Anfrage holt das Fenster nach vorn, und der Nutzer vergleicht den sechsstelligen Code mit
 * dem der anfragenden App. Bestätigen oder Ablehnen geht nur hier; nach 120 s verfällt die Anfrage.
 * Gekoppelt hat eine App danach noch keine Rechte – die vergibt der Nutzer in den Integrationen.
 */
export const KopplungsDialog: React.FC = () => {
  const { t } = useTranslation();
  const [offen, setOffen] = useState<SprachKopplung[]>([]);
  const [jetzt, setJetzt] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const r = await commands.sprachdienstKopplungen();
      if (r.status === "ok") setOffen(r.data ?? []);
    } catch {
      /* ohne Anfrage bleibt der Dialog zu */
    }
  }, []);

  useEffect(() => {
    void reload();
    const un = listen("sprachdienst-kopplung", () => void reload());
    return () => {
      void un.then((f) => f());
    };
  }, [reload]);

  const first = offen.length > 0 ? offen[0] : null;
  const firstId = first?.id ?? null;

  // Sekundentakt nur, solange eine Anfrage offen ist (Countdown und Verfall).
  useEffect(() => {
    if (!firstId) return;
    setJetzt(Date.now());
    const id = setInterval(() => setJetzt(Date.now()), 1000);
    return () => clearInterval(id);
  }, [firstId]);

  const verfallen = first != null && first.bis_ms <= jetzt;
  useEffect(() => {
    if (verfallen) void reload();
  }, [verfallen, reload]);

  const decide = async (bestaetigen: boolean) => {
    if (!first) return;
    setBusy(true);
    setError(null);
    try {
      const r = await commands.sprachdienstKopplungEntscheiden(
        first.id,
        bestaetigen,
      );
      if (r.status === "error") {
        setError(errorText(r.error) || t("integrations.errors.generic"));
      } else if (!r.data) {
        setError(t("integrations.sprachdienst.pairing.expired"));
      }
    } catch (e) {
      setError(errorText(e) || t("integrations.errors.generic"));
    }
    await reload();
    setBusy(false);
  };

  const seconds = first
    ? Math.max(0, Math.ceil((first.bis_ms - jetzt) / 1000))
    : 0;

  return (
    <Dialog
      open={first != null && !verfallen}
      onOpenChange={(open) => {
        if (!open && !busy) void decide(false);
      }}
      title={t("integrations.sprachdienst.pairing.title")}
      closeLabel={t("integrations.sprachdienst.pairing.reject")}
      closeOnBackdrop={false}
      footer={
        <>
          <Button
            size="sm"
            disabled={busy || verfallen}
            onClick={() => void decide(true)}
            data-testid="kopplung-confirm"
          >
            {t("integrations.sprachdienst.pairing.confirm")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => void decide(false)}
            data-testid="kopplung-reject"
          >
            {t("integrations.sprachdienst.pairing.reject")}
          </Button>
        </>
      }
    >
      {first && (
        <div className="space-y-3" data-testid="kopplung-dialog">
          <p className="text-sm">
            {t("integrations.sprachdienst.pairing.body", {
              name: first.anzeige,
            })}
          </p>
          <div className="rounded-lg border border-mid-gray/30 p-3 text-center">
            <p className="text-xs text-text-muted">
              {t("integrations.sprachdienst.pairing.codeLabel")}
            </p>
            <p
              className="font-mono text-3xl font-semibold tracking-widest"
              data-testid="kopplung-code"
              aria-label={first.code.split("").join(" ")}
            >
              {gruppiert(first.code)}
            </p>
          </div>
          <p
            className="text-xs text-text-muted"
            role="timer"
            data-testid="kopplung-restzeit"
          >
            {t("integrations.sprachdienst.pairing.remaining", { seconds })}
          </p>
          <p className="text-xs text-text-muted">
            {t("integrations.sprachdienst.pairing.afterwards")}
          </p>
          {offen.length > 1 && (
            <p className="text-xs" data-testid="kopplung-weitere">
              {t("integrations.sprachdienst.pairing.more", {
                count: offen.length - 1,
              })}
            </p>
          )}
          {error && (
            <p className="text-sm text-red-400" role="alert">
              {error}
            </p>
          )}
        </div>
      )}
    </Dialog>
  );
};
