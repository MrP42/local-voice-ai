import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  commands,
  type SprachZugang,
  type SprachdienstStatus,
} from "@/bindings";
import { Button } from "../ui/Button";
import { ToggleSwitch } from "../ui/ToggleSwitch";
import { errorText } from "./model";

type Confirm = { id: string; kind: "revoke" | "delete" } | null;

const stamp = (ms: number, language: string): string =>
  new Date(ms).toLocaleString(language, {
    dateStyle: "medium",
    timeStyle: "short",
  });

/**
 * Sprachdienst für andere WAI-Apps (ADR-0021): Opt-in-Schalter, Zustand und die gekoppelten
 * Apps mit Rechten (Hören, Sprechen), Widerruf und Entfernen. Gekoppelt wird nur im
 * Kopplungsdialog (`KopplungsDialog`), nie hier und nie über den Dienst selbst.
 */
export const SprachdienstPanel: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<SprachdienstStatus | null>(null);
  const [zugaenge, setZugaenge] = useState<SprachZugang[] | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<Confirm>(null);

  const reload = useCallback(async () => {
    try {
      const [s, z] = await Promise.all([
        commands.sprachdienstStatus(),
        commands.sprachdienstZugaenge(),
      ]);
      if (s.status === "ok" && z.status === "ok") {
        setStatus(s.data);
        setZugaenge(z.data ?? []);
        setLoadFailed(false);
      } else {
        setLoadFailed(true);
      }
    } catch {
      setLoadFailed(true);
    }
  }, []);

  useEffect(() => {
    void reload();
    const un = listen("sprachdienst-zugaenge", () => void reload());
    return () => {
      void un.then((f) => f());
    };
  }, [reload]);

  const fail = (raw: unknown) =>
    setError(errorText(raw) || t("integrations.errors.generic"));

  const act = async (
    action: () => Promise<
      { status: "ok" } | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    setError(null);
    try {
      const result = await action();
      if (result.status === "error") fail(result.error);
    } catch (e) {
      fail(e);
    }
    setConfirm(null);
    setBusy(false);
    await reload();
  };

  const aktiv = status?.aktiv ?? false;

  return (
    <section
      className="space-y-3"
      aria-labelledby="int-sec-sprache"
      data-testid="sprachdienst"
    >
      <div>
        <h2 id="int-sec-sprache" className="text-sm font-semibold">
          {t("integrations.sections.sprachdienst")}
        </h2>
        <p className="text-xs text-text-muted">
          {t("integrations.sections.sprachdienstHint")}
        </p>
      </div>

      <div className="rounded-lg border border-mid-gray/20">
        <ToggleSwitch
          checked={aktiv}
          disabled={busy || status === null}
          onChange={(an) =>
            void act(async () => {
              const r = await commands.sprachdienstAktivSetzen(an);
              return r.status === "ok" ? { status: "ok" as const } : r;
            })
          }
          label={t("integrations.sprachdienst.switch.label")}
          description={t("integrations.sprachdienst.switch.description")}
          descriptionMode="inline"
          grouped
          testId="sprachdienst-switch"
        />
      </div>

      {status && (
        <div
          className={`rounded-lg px-3 py-2 text-xs ${
            status.laeuft ? "bg-green-500/10" : "bg-mid-gray/10"
          }`}
          role="status"
          data-testid="sprachdienst-status"
          data-running={status.laeuft}
        >
          <p className="font-medium">
            {status.laeuft
              ? t("integrations.sprachdienst.status.running")
              : aktiv
                ? t("integrations.sprachdienst.status.stopped")
                : t("integrations.sprachdienst.status.off")}
          </p>
          {status.endpunkt && (
            <p className="break-all text-text-muted">
              {t("integrations.sprachdienst.status.endpoint", {
                name: status.endpunkt,
              })}
            </p>
          )}
          {status.fehler && (
            <p className="break-words" data-testid="sprachdienst-error">
              {t("integrations.sprachdienst.status.reason", {
                error: status.fehler,
              })}
            </p>
          )}
          {status.laeuft && (
            <p className="text-text-muted">
              {t("integrations.sprachdienst.status.safe")}
            </p>
          )}
        </div>
      )}

      {error && (
        <p
          className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-red-400"
          role="alert"
          data-testid="sprachdienst-action-error"
        >
          {error}
        </p>
      )}
      {loadFailed && (
        <p className="text-sm text-red-400" role="alert">
          {t("integrations.sprachdienst.loadError")}
        </p>
      )}
      {zugaenge && zugaenge.length === 0 && !loadFailed && (
        <p className="text-sm text-text-muted" data-testid="sprachdienst-empty">
          {t("integrations.sprachdienst.empty")}
        </p>
      )}

      <ul className="space-y-3">
        {(zugaenge ?? []).map((z) => {
          const revoked = z.widerrufen_ms != null;
          const isConfirming = confirm?.id === z.zugang;
          return (
            <li
              key={z.zugang}
              className="space-y-3 rounded-lg border border-mid-gray/20 p-3"
              data-testid="sprachdienst-zugang"
              data-zugang={z.zugang}
              data-revoked={revoked}
            >
              <div className="flex flex-wrap items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="break-words text-sm font-medium">
                    {z.anzeige}
                    <span
                      className={`ms-2 rounded-full px-2 py-0.5 text-xs font-normal ${
                        revoked
                          ? "bg-mid-gray/20 text-text-muted"
                          : "bg-green-500/15 text-text"
                      }`}
                      data-testid="sprachdienst-zugang-status"
                    >
                      {revoked
                        ? t("integrations.sprachdienst.client.revoked")
                        : t("integrations.sprachdienst.client.active")}
                    </span>
                  </p>
                  <p className="text-xs text-text-muted">
                    {t("integrations.sprachdienst.client.created", {
                      time: stamp(z.erstellt_ms, i18n.language),
                    })}
                    {" · "}
                    <span data-testid="sprachdienst-zugang-last-used">
                      {z.zuletzt_ms != null
                        ? t("integrations.sprachdienst.client.lastUsed", {
                            time: stamp(z.zuletzt_ms, i18n.language),
                          })
                        : t("integrations.sprachdienst.client.neverUsed")}
                    </span>
                  </p>
                  {revoked && z.widerrufen_ms != null && (
                    <p className="text-xs text-text-muted">
                      {t("integrations.sprachdienst.client.revokedAt", {
                        time: stamp(z.widerrufen_ms, i18n.language),
                      })}
                    </p>
                  )}
                </div>
                {!isConfirming && (
                  <div className="flex flex-wrap gap-2">
                    {!revoked && (
                      <Button
                        variant="secondary"
                        size="sm"
                        disabled={busy}
                        onClick={() =>
                          setConfirm({ id: z.zugang, kind: "revoke" })
                        }
                        data-testid="sprachdienst-revoke"
                      >
                        {t("integrations.sprachdienst.client.revoke")}
                      </Button>
                    )}
                    <Button
                      variant="danger-ghost"
                      size="sm"
                      disabled={busy}
                      onClick={() =>
                        setConfirm({ id: z.zugang, kind: "delete" })
                      }
                      data-testid="sprachdienst-delete"
                    >
                      {t("integrations.sprachdienst.client.delete")}
                    </Button>
                  </div>
                )}
              </div>

              {isConfirming && confirm && (
                <div
                  className="flex flex-wrap items-center gap-2"
                  data-testid="sprachdienst-confirm"
                  data-kind={confirm.kind}
                >
                  <span className="text-sm">
                    {t(
                      confirm.kind === "revoke"
                        ? "integrations.sprachdienst.client.revokeConfirm"
                        : "integrations.sprachdienst.client.deleteConfirm",
                      { name: z.anzeige },
                    )}
                  </span>
                  <Button
                    variant="danger"
                    size="sm"
                    disabled={busy}
                    onClick={() =>
                      void act(() =>
                        confirm.kind === "revoke"
                          ? commands.sprachdienstWiderrufen(z.zugang)
                          : commands.sprachdienstEntfernen(z.zugang),
                      )
                    }
                    data-testid="sprachdienst-confirm-yes"
                  >
                    {t(
                      confirm.kind === "revoke"
                        ? "integrations.sprachdienst.client.revokeYes"
                        : "integrations.sprachdienst.client.deleteYes",
                    )}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={busy}
                    onClick={() => setConfirm(null)}
                    data-testid="sprachdienst-confirm-no"
                  >
                    {t("integrations.cancel")}
                  </Button>
                </div>
              )}

              {!revoked && (
                <div className="space-y-1" data-testid="sprachdienst-rechte">
                  <p className="text-xs text-text-muted">
                    {t("integrations.sprachdienst.client.rightsHint")}
                  </p>
                  {(["hoeren", "sprechen"] as const).map((recht) => (
                    <label
                      key={recht}
                      className="flex cursor-pointer items-center gap-2 text-sm"
                    >
                      <input
                        type="checkbox"
                        checked={z[recht]}
                        disabled={busy}
                        onChange={(e) =>
                          void act(async () => {
                            const r = await commands.sprachdienstRechtSetzen(
                              z.zugang,
                              recht,
                              e.target.checked,
                            );
                            return r.status === "ok"
                              ? { status: "ok" as const }
                              : r;
                          })
                        }
                        className="accent-logo-primary"
                        data-testid={`sprachdienst-recht-${recht}`}
                      />
                      <span>
                        {t(`integrations.sprachdienst.rights.${recht}`)}
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
};
