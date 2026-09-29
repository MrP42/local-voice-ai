import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, EyeOff } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands, type CalendarSource } from "@/bindings";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { Input } from "../../ui/Input";
import { useSettings } from "../../../hooks/useSettings";

interface CalendarConnectDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdded: (source: CalendarSource) => void;
}

/** Anleitung von Microsoft: App in Entra registrieren. */
const ENTRA_REGISTER_URL =
  "https://learn.microsoft.com/entra/identity-platform/quickstart-register-app";

/** Anwendungs-(Client-)ID: eine GUID (8-4-4-4-12). Das Backend prueft erneut. */
const GUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Kalender verbinden (M5-P5b, P5f). Zwei Wege:
 *
 * - Microsoft-Anmeldung (P5f): Outlook/Microsoft 365 ueber Microsoft Graph, mit
 *   Teilnehmenden. Es gibt keine eingebaute Client-ID (E14): der Nutzer traegt die
 *   Anwendungs-ID seiner eigenen Entra-App ein (Feld hier, gespeichert in den
 *   Einstellungen); ohne gueltige ID ist der Knopf gesperrt und ein Hinweis erklaert
 *   die Registrierung. Die Anmeldung laeuft im Browser und kann abgebrochen werden.
 * - ICS-Adresse (P5b): Der Probeabruf laeuft im Backend VOR dem Speichern; liefert
 *   die Adresse keinen lesbaren Kalender, entsteht nichts. Die Adresse ist ein
 *   Geheimnis (Lesezugriff auf den ganzen Kalender) und bleibt maskiert; nach dem
 *   Speichern zeigt die Oberflaeche nur noch den Host.
 */
export const CalendarConnectDialog: React.FC<CalendarConnectDialogProps> = ({
  open,
  onOpenChange,
  onAdded,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const savedClientId = getSetting("calendar_graph_client_id") ?? "";
  const savedTenant = getSetting("calendar_graph_tenant") ?? "";

  const [label, setLabel] = useState("");
  const [url, setUrl] = useState("");
  const [reveal, setReveal] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [clientId, setClientId] = useState("");
  const [tenant, setTenant] = useState("");
  const [graphBusy, setGraphBusy] = useState(false);
  const [graphError, setGraphError] = useState<string | null>(null);
  const cancelled = useRef(false);

  useEffect(() => {
    if (open) {
      // Die gespeicherten Werte vorbelegen.
      setClientId(savedClientId);
      setTenant(savedTenant);
      return;
    }
    // Nichts vom Geheimnis bleibt im Zustand, wenn der Dialog zu ist.
    setLabel("");
    setUrl("");
    setReveal(false);
    setBusy(false);
    setError(null);
    setGraphBusy(false);
    setGraphError(null);
  }, [open]);

  const clientIdValid = GUID.test(clientId.trim());
  const anyBusy = busy || graphBusy;

  const connect = async () => {
    if (!url.trim() || anyBusy) return;
    setBusy(true);
    setError(null);
    const result = await commands.calendarSourceAddIcs(
      label.trim(),
      url.trim(),
    );
    setBusy(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    onAdded(result.data);
    onOpenChange(false);
  };

  const signIn = async () => {
    if (!clientIdValid || anyBusy) return;
    cancelled.current = false;
    setGraphBusy(true);
    setGraphError(null);
    const id = clientId.trim().toLowerCase();
    const tn = tenant.trim();
    // Das Backend liest Client-ID und Verzeichnis aus den Einstellungen.
    if (id !== savedClientId) {
      await updateSetting("calendar_graph_client_id", id);
    }
    if (tn !== savedTenant) {
      await updateSetting("calendar_graph_tenant", tn === "" ? null : tn);
    }
    const result = await commands.calendarGraphSignIn();
    setGraphBusy(false);
    if (result.status === "error") {
      // Ein Abbruch durch den Nutzer ist keine Fehlermeldung.
      if (!cancelled.current) setGraphError(result.error);
      return;
    }
    onAdded(result.data);
    onOpenChange(false);
  };

  const cancelSignIn = async () => {
    cancelled.current = true;
    await commands.calendarGraphCancelSignIn();
  };

  const toggleLabel = reveal
    ? t("meetings.calendar.dialog.hide")
    : t("meetings.calendar.dialog.show");

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!anyBusy) onOpenChange(next);
      }}
      title={t("meetings.calendar.dialog.title")}
      closeLabel={t("meetings.calendar.cancel")}
      footer={
        <>
          <Button
            variant="secondary"
            onClick={() => onOpenChange(false)}
            disabled={anyBusy}
          >
            {t("meetings.calendar.cancel")}
          </Button>
          <Button
            onClick={() => void connect()}
            disabled={anyBusy || !url.trim()}
            data-testid="calendar-connect-submit"
          >
            {busy
              ? t("meetings.calendar.dialog.checking")
              : t("meetings.calendar.dialog.connect")}
          </Button>
        </>
      }
    >
      <div className="space-y-4" data-testid="calendar-dialog">
        <section className="space-y-3" data-testid="calendar-graph">
          <div>
            <h3 className="text-sm font-semibold">
              {t("meetings.calendar.graph.title")}
            </h3>
            <p className="text-xs text-text/60">
              {t("meetings.calendar.graph.intro")}
            </p>
          </div>

          <label className="block space-y-1">
            <span className="text-sm font-medium">
              {t("meetings.calendar.graph.clientId")}
            </span>
            <Input
              type="text"
              value={clientId}
              onChange={(e) => setClientId(e.target.value)}
              placeholder="00000000-0000-0000-0000-000000000000"
              disabled={anyBusy}
              autoComplete="off"
              spellCheck={false}
              className="w-full"
              data-testid="calendar-graph-client-id"
            />
          </label>
          <label className="block space-y-1">
            <span className="text-sm font-medium">
              {t("meetings.calendar.graph.tenant")}
            </span>
            <Input
              type="text"
              value={tenant}
              onChange={(e) => setTenant(e.target.value)}
              placeholder="common"
              disabled={anyBusy}
              autoComplete="off"
              spellCheck={false}
              className="w-full"
              data-testid="calendar-graph-tenant"
            />
            <span className="block text-xs text-text/60">
              {t("meetings.calendar.graph.tenantHint")}
            </span>
          </label>

          {!clientIdValid && (
            <div
              className="rounded-lg border border-mid-gray/20 p-3 text-sm text-text/80"
              data-testid="calendar-graph-hint"
            >
              <p>
                {clientId.trim() === ""
                  ? t("meetings.calendar.graph.noClientId")
                  : t("meetings.calendar.graph.invalidClientId")}
              </p>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => void openUrl(ENTRA_REGISTER_URL)}
                data-testid="calendar-graph-help-link"
              >
                {t("meetings.calendar.graph.helpLink")}
              </Button>
            </div>
          )}

          {graphError && (
            <div data-testid="calendar-graph-error">
              <Alert variant="error">{graphError}</Alert>
            </div>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <Button
              onClick={() => void signIn()}
              disabled={!clientIdValid || anyBusy}
              data-testid="calendar-graph-signin"
            >
              {graphBusy
                ? t("meetings.calendar.graph.waiting")
                : t("meetings.calendar.graph.signIn")}
            </Button>
            {graphBusy && (
              <Button
                variant="secondary"
                onClick={() => void cancelSignIn()}
                data-testid="calendar-graph-cancel"
              >
                {t("meetings.calendar.graph.cancelSignIn")}
              </Button>
            )}
          </div>
        </section>

        <div
          className="flex items-center gap-3 text-xs text-text/50"
          role="separator"
        >
          <span className="h-px flex-1 bg-mid-gray/20" />
          {t("meetings.calendar.graph.or")}
          <span className="h-px flex-1 bg-mid-gray/20" />
        </div>

        <section className="space-y-3" data-testid="calendar-ics">
          <h3 className="text-sm font-semibold">
            {t("meetings.calendar.dialog.icsTitle")}
          </h3>
          <details className="rounded-lg border border-mid-gray/20 p-3 text-sm">
            <summary className="cursor-pointer font-medium">
              {t("meetings.calendar.dialog.helpTitle")}
            </summary>
            <div className="mt-2 space-y-2 text-text/80">
              <div>
                <p className="font-medium">
                  {t("meetings.calendar.dialog.outlookTitle")}
                </p>
                <p>{t("meetings.calendar.dialog.outlookSteps")}</p>
              </div>
              <div>
                <p className="font-medium">
                  {t("meetings.calendar.dialog.googleTitle")}
                </p>
                <p>{t("meetings.calendar.dialog.googleSteps")}</p>
              </div>
              <div>
                <p className="font-medium">
                  {t("meetings.calendar.dialog.otherTitle")}
                </p>
                <p>{t("meetings.calendar.dialog.otherSteps")}</p>
              </div>
            </div>
          </details>

          <label className="block space-y-1">
            <span className="text-sm font-medium">
              {t("meetings.calendar.dialog.name")}
            </span>
            <Input
              type="text"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder={t("meetings.calendar.dialog.namePlaceholder")}
              disabled={anyBusy}
              className="w-full"
              data-testid="calendar-name"
            />
          </label>

          <div className="space-y-1">
            <label htmlFor="calendar-url" className="text-sm font-medium">
              {t("meetings.calendar.dialog.url")}
            </label>
            <div className="flex items-center gap-2">
              <Input
                id="calendar-url"
                type={reveal ? "text" : "password"}
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder={t("meetings.calendar.dialog.urlPlaceholder")}
                disabled={anyBusy}
                autoComplete="off"
                spellCheck={false}
                className="flex-1"
                data-testid="calendar-url"
              />
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setReveal((v) => !v)}
                aria-label={toggleLabel}
                title={toggleLabel}
                data-testid="calendar-url-toggle"
              >
                {reveal ? (
                  <EyeOff width={16} height={16} />
                ) : (
                  <Eye width={16} height={16} />
                )}
              </Button>
            </div>
            <p className="text-xs text-text/60">
              {t("meetings.calendar.dialog.privacy")}
            </p>
          </div>

          {error && (
            <div data-testid="calendar-connect-error">
              <Alert variant="error">{error}</Alert>
            </div>
          )}
        </section>
      </div>
    </Dialog>
  );
};
