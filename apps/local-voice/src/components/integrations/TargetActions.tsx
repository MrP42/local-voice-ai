import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type IntegrationView, type WissenHit } from "@/bindings";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { configText, errorText } from "./model";

interface TargetActionsProps {
  view: IntegrationView;
  /** Nach einer Aktion hat das Backend `last_ok_at`/`last_error` gesetzt. */
  onTouched: () => void;
}

/**
 * Aktionen, die zu einer Art gehoeren: beim SMTP-Postfach die Testmail an die
 * eigene Absenderadresse, bei der Wissensbasis eine Probesuche. Beides sind
 * Schritte des Nutzers (ohne Freigabe) und stehen im Protokoll.
 */
export const TargetActions: React.FC<TargetActionsProps> = ({
  view,
  onTouched,
}) => {
  const { t } = useTranslation();
  const { integration } = view;
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(
    null,
  );
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<WissenHit[] | null>(null);

  const explain = (raw: string) => {
    const text = t(`integrations.errors.${raw}`, { defaultValue: "" });
    return text || raw || t("integrations.errors.generic");
  };

  const sendTestMail = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await commands.integrationSendTestMail(integration.id);
      setMessage(
        result.status === "ok"
          ? {
              ok: true,
              text: t("integrations.detail.mailSent", {
                address: configText(integration.config_json, "from_address"),
              }),
            }
          : { ok: false, text: explain(errorText(result.error)) },
      );
    } catch (e) {
      setMessage({ ok: false, text: explain(errorText(e)) });
    }
    setBusy(false);
    onTouched();
  };

  const search = async () => {
    setBusy(true);
    setMessage(null);
    setHits(null);
    try {
      const result = await commands.wissenSuchen(
        integration.id,
        query.trim(),
        5,
        null,
      );
      if (result.status === "ok") setHits(result.data ?? []);
      else setMessage({ ok: false, text: explain(errorText(result.error)) });
    } catch (e) {
      setMessage({ ok: false, text: explain(errorText(e)) });
    }
    setBusy(false);
    onTouched();
  };

  if (integration.kind === "smtp") {
    return (
      <div className="space-y-1" data-testid="smtp-actions">
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="secondary"
            size="sm"
            disabled={busy || !integration.enabled}
            onClick={() => void sendTestMail()}
            data-testid="smtp-test-mail"
          >
            {busy
              ? t("integrations.detail.mailTesting")
              : t("integrations.detail.mailTest")}
          </Button>
          {message && (
            <span
              role={message.ok ? "status" : "alert"}
              data-testid="smtp-test-mail-result"
              className={`text-sm break-words ${message.ok ? "text-status-green" : "text-status-red"}`}
            >
              {message.text}
            </span>
          )}
        </div>
        <p className="text-xs text-text-muted">
          {t("integrations.detail.mailPending")}
        </p>
      </div>
    );
  }

  if (integration.kind === "wissen") {
    return (
      <div className="space-y-2" data-testid="wissen-actions">
        <h4 className="text-sm font-medium">
          {t("integrations.detail.searchTitle")}
        </h4>
        <form
          className="flex flex-wrap items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (query.trim() && !busy) void search();
          }}
        >
          <label className="sr-only" htmlFor="wissen-query">
            {t("integrations.detail.searchPlaceholder")}
          </label>
          <Input
            id="wissen-query"
            className="min-w-0 flex-1 basis-48"
            value={query}
            maxLength={2000}
            placeholder={t("integrations.detail.searchPlaceholder")}
            onChange={(e) => setQuery(e.target.value)}
            data-testid="wissen-query"
          />
          <Button
            type="submit"
            variant="secondary"
            size="sm"
            disabled={busy || !query.trim() || !integration.enabled}
            data-testid="wissen-search"
          >
            {busy
              ? t("integrations.detail.searching")
              : t("integrations.detail.searchButton")}
          </Button>
        </form>
        {message && (
          <p
            role="alert"
            data-testid="wissen-search-error"
            className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-status-red break-words"
          >
            {message.text}
          </p>
        )}
        {hits && (
          <div data-testid="wissen-hits" className="space-y-2">
            <p className="text-xs text-text-muted">
              {hits.length === 0
                ? t("integrations.detail.searchNone")
                : t("integrations.detail.searchHits", { count: hits.length })}
            </p>
            <ul className="space-y-2">
              {hits.map((hit, i) => (
                <li
                  key={`${hit.document_id ?? hit.path}-${i}`}
                  className="rounded-lg border border-mid-gray/20 p-2 text-sm"
                  data-testid="wissen-hit"
                >
                  <div className="font-medium break-words">{hit.title}</div>
                  <div className="text-xs text-text-muted break-all">
                    {hit.path}
                    {hit.area ? ` · ${hit.area}` : ""}
                  </div>
                  {hit.snippet && (
                    <p className="mt-1 break-words text-text-muted">
                      {hit.snippet}
                    </p>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    );
  }

  return null;
};
