import React, { useCallback, useEffect, useRef, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { commands, type SprachKopplung, type SprachZugang } from "@/bindings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { kopplungsWarnungen } from "./kopplung";
import { errorText } from "./model";

/** So lange ist „Koppeln“ nach dem Erscheinen oder einem Wechsel der Anfrage gesperrt. */
const SPERRE_MS = 1000;

/** „123456“ als „123 456“, damit der Vergleich mit der anderen App leichter fällt. */
const gruppiert = (code: string): string =>
  code.length === 6 ? `${code.slice(0, 3)} ${code.slice(3)}` : code;

/**
 * Kopplungsanfrage einer App an den Sprachdienst (ADR-0021). Der Dialog steht global in der App:
 * eine Anfrage holt das Fenster nach vorn, und der Nutzer vergleicht den sechsstelligen Code mit
 * dem der anfragenden App. Bestätigen oder Ablehnen geht nur hier; nach 120 s verfällt die Anfrage.
 * Gekoppelt hat eine App danach noch keine Rechte – die vergibt der Nutzer in den Integrationen.
 *
 * Die Entscheidung gilt immer der Anfrage, die der Dialog gerade zeigt (`gezeigt`), nicht der,
 * die in der Liste inzwischen vorn steht. Wechselt die gezeigte Anfrage, ist „Koppeln“ eine
 * Sekunde gesperrt, damit ein Klick auf die vorige nicht die neue bestätigt.
 */
export const KopplungsDialog: React.FC = () => {
  const { t } = useTranslation();
  const [offen, setOffen] = useState<SprachKopplung[]>([]);
  const [zugaenge, setZugaenge] = useState<SprachZugang[]>([]);
  const [gezeigt, setGezeigt] = useState<SprachKopplung | null>(null);
  const [gesperrt, setGesperrt] = useState(false);
  const [jetzt, setJetzt] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sperrTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const reload = useCallback(async () => {
    try {
      const [k, z] = await Promise.all([
        commands.sprachdienstKopplungen(),
        commands.sprachdienstZugaenge(),
      ]);
      if (k.status === "ok") setOffen(k.data ?? []);
      if (z.status === "ok") setZugaenge(z.data ?? []);
    } catch {
      /* ohne Anfrage bleibt der Dialog zu */
    }
  }, []);

  useEffect(() => {
    void reload();
    const un = listen("sprachdienst-kopplung", () => void reload());
    return () => {
      void un.then((f) => f());
      if (sperrTimer.current) clearTimeout(sperrTimer.current);
    };
  }, [reload]);

  // Die gezeigte Anfrage bleibt, solange es sie gibt; sonst die erste der Liste, mit Sperre.
  useEffect(() => {
    setGezeigt((aktuell) => {
      const noch = aktuell && offen.find((k) => k.id === aktuell.id);
      if (noch) return noch;
      return offen.length > 0 ? offen[0] : null;
    });
  }, [offen]);

  const gezeigtId = gezeigt?.id ?? null;
  useEffect(() => {
    if (sperrTimer.current) clearTimeout(sperrTimer.current);
    if (!gezeigtId) {
      setGesperrt(false);
      return;
    }
    setGesperrt(true);
    sperrTimer.current = setTimeout(() => setGesperrt(false), SPERRE_MS);
    setError(null);
  }, [gezeigtId]);

  // Sekundentakt nur, solange eine Anfrage gezeigt wird (Countdown und Verfall).
  useEffect(() => {
    if (!gezeigtId) return;
    setJetzt(Date.now());
    const id = setInterval(() => setJetzt(Date.now()), 1000);
    return () => clearInterval(id);
  }, [gezeigtId]);

  const verfallen = gezeigt != null && gezeigt.bis_ms <= jetzt;
  useEffect(() => {
    if (verfallen) void reload();
  }, [verfallen, reload]);

  const decide = async (bestaetigen: boolean) => {
    if (!gezeigt) return;
    if (bestaetigen && gesperrt) return;
    setBusy(true);
    setError(null);
    try {
      const r = await commands.sprachdienstKopplungEntscheiden(
        gezeigt.id,
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

  const seconds = gezeigt
    ? Math.max(0, Math.ceil((gezeigt.bis_ms - jetzt) / 1000))
    : 0;
  const warnungen = gezeigt
    ? kopplungsWarnungen(
        gezeigt,
        zugaenge,
        offen.filter((k) => k.id !== gezeigt.id),
      )
    : [];

  return (
    <Dialog
      open={gezeigt != null && !verfallen}
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
            disabled={busy || verfallen || gesperrt}
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
      {gezeigt && (
        <div
          className="space-y-3"
          data-testid="kopplung-dialog"
          data-kopplung-id={gezeigt.id}
        >
          <p className="text-sm">
            <Trans
              i18nKey="integrations.sprachdienst.pairing.body"
              values={{ name: gezeigt.anzeige }}
              components={{
                name: (
                  <bdi className="font-semibold" data-testid="kopplung-name" />
                ),
              }}
            />
          </p>
          <p className="text-xs text-text-muted">
            {t("integrations.sprachdienst.pairing.client")}{" "}
            <code className="break-all" data-testid="kopplung-client">
              {gezeigt.client}
            </code>
          </p>
          {warnungen.length > 0 && (
            <ul
              className="space-y-1 rounded-lg bg-amber-500/10 px-3 py-2 text-xs"
              role="alert"
              data-testid="kopplung-warnungen"
            >
              {warnungen.map((w) => (
                <li key={w} data-warnung={w}>
                  {t(`integrations.sprachdienst.pairing.warnings.${w}`)}
                </li>
              ))}
            </ul>
          )}
          <div className="rounded-lg border border-mid-gray/30 p-3 text-center">
            <p className="text-xs text-text-muted">
              {t("integrations.sprachdienst.pairing.codeLabel")}
            </p>
            <p
              className="font-mono text-3xl font-semibold tracking-widest"
              data-testid="kopplung-code"
              aria-label={gezeigt.code.split("").join(" ")}
            >
              {gruppiert(gezeigt.code)}
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
