import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { Cloud, CloudOff, CloudUpload, LogOut, RefreshCw } from "lucide-react";
import {
  commands,
  type AnmeldeWahl,
  type Entscheidung,
  type HubStatus,
  type SyncStatus,
} from "@/bindings";
import { SettingsGroup } from "../ui/SettingsGroup";
import { SettingContainer } from "../ui/SettingContainer";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { relativeTime } from "./tts/WorkspaceSidebars";
import { AnmeldungDatenDialog } from "./AnmeldungDatenDialog";
import { AbmeldenDialog } from "./AbmeldenDialog";
import { SicherungenListe } from "./SicherungenListe";

/**
 * Konto & Geräte: Anmelden am Hub, Stand des Abgleichs, Abmelden. Sitzt im
 * Reiter „Konto & Geräte" (seit ADR-0024, vorher „Allgemein"). Der
 * Schlüssel entsteht aus dem Passwort auf diesem Gerät; der Server sieht
 * nur Chiffrate (Spec 2026-09-14, Abschnitt 3).
 */
export const SyncAccountCard: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [hub, setHub] = useState<HubStatus | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [deviceName, setDeviceName] = useState("");
  const [url, setUrl] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  // Wartende Anmeldung mit lokalem Bestand (Spec 2026-11-08, U3).
  const [entscheidung, setEntscheidung] = useState<Entscheidung | null>(null);
  const [abmelden, setAbmelden] = useState(false);
  // Neue Sicherung möglich: Liste neu laden.
  const [sicherungStand, setSicherungStand] = useState(0);

  const refresh = useCallback(async () => {
    const s = await commands.syncStatus();
    // Ohne Backend (Tests, alter Kern) kommt nichts zurueck: abgemeldet zeigen.
    if (!s) return;
    setStatus(s);
    if (s.anmeldung_offen) {
      const e = await commands.syncAnmeldungOffen();
      if (e?.status === "ok")
        setEntscheidung((alt) => (e.data ? (alt ?? e.data) : null));
    }
    if (s.connected) {
      const h = await commands.syncHubStatus();
      setHub(h.status === "ok" ? h.data : null);
    } else {
      setHub(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
    void commands.syncDefaultDeviceName().then((name) => {
      if (typeof name === "string") setDeviceName((current) => current || name);
    });
    const unlisten = listen<SyncStatus>("sync-status", (event) => {
      setStatus(event.payload);
      // Wiederanlauf mit wartender Anmeldung: Modal erneut zeigen (U11).
      if (event.payload.anmeldung_offen) {
        void commands.syncAnmeldungOffen().then((e) => {
          if (e?.status === "ok" && e.data)
            setEntscheidung((alt) => alt ?? e.data);
        });
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [refresh]);

  const login = async () => {
    setBusy(true);
    const result = await commands.syncLogin(
      email,
      password,
      deviceName,
      url.trim() ? url.trim() : null,
    );
    setBusy(false);
    if (result.status === "error") {
      toast.error(t("sync.loginFailed"), { description: result.error });
      return;
    }
    setPassword("");
    setStatus(result.data.status);
    if (result.data.entscheidung) {
      setEntscheidung(result.data.entscheidung);
      return;
    }
    toast.success(t("sync.loggedIn"));
    void refresh();
  };

  const entschieden = (s: SyncStatus, wahl: AnmeldeWahl | "abbrechen") => {
    setEntscheidung(null);
    setStatus(s);
    setSicherungStand((n) => n + 1);
    if (wahl === "abbrechen") {
      toast.success(t("sync.anmeldung.abgebrochen"));
    } else {
      toast.success(t(`sync.anmeldung.erledigt.${wahl}`));
    }
    void refresh();
  };

  const abgemeldet = (s: SyncStatus, entfernt: boolean) => {
    setStatus(s);
    setHub(null);
    setSicherungStand((n) => n + 1);
    toast.success(entfernt ? t("sync.abmelden.entfernt") : t("sync.loggedOut"));
  };

  const nurLokalUebernehmen = async () => {
    setBusy(true);
    const result = await commands.syncNurLokalUebernehmen();
    setBusy(false);
    if (result.status === "error") {
      toast.error(t("sync.failed"), { description: result.error });
      return;
    }
    setStatus(result.data);
    setSicherungStand((n) => n + 1);
    toast.success(t("sync.nurLokal.uebernommen"));
    void refresh();
  };

  const syncNow = async () => {
    setBusy(true);
    const result = await commands.syncNow();
    setBusy(false);
    if (result.status === "ok") {
      setStatus(result.data);
      if (result.data.last_error) {
        toast.error(t("sync.failed"), { description: result.data.last_error });
      } else {
        toast.success(t("sync.done"));
      }
      void refresh();
    }
  };

  const connected = status?.connected ?? false;
  const nurLokal = status?.nur_lokal ?? 0;

  const hinweise = (
    <>
      {status?.widerruf_offen && (
        <p
          className="px-4 pb-2 text-sm text-orange-500"
          data-testid="sync-widerruf"
        >
          {t("sync.widerrufOffen")}
        </p>
      )}
      {status?.uebergang_fehler && (
        <div
          className="px-4 pb-2 text-sm flex flex-wrap items-center gap-2"
          role="alert"
        >
          <span className="text-orange-500">{status.uebergang_fehler}</span>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => void syncNow()}
            disabled={busy}
          >
            {t("sync.erneutPruefen")}
          </Button>
        </div>
      )}
    </>
  );

  return (
    <SettingsGroup
      title={t("sync.groupTitle")}
      description={t("sync.groupDescription")}
    >
      {!connected ? (
        <div className="px-4 py-3 space-y-3" data-testid="sync-login">
          <p className="text-sm text-text/70 flex items-start gap-2">
            <CloudOff
              width={16}
              height={16}
              className="mt-0.5 shrink-0"
              aria-hidden="true"
            />
            <span>{t("sync.loggedOutHint")}</span>
          </p>
          <div className="grid gap-2 sm:grid-cols-2">
            <Input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder={t("sync.email")}
              aria-label={t("sync.email")}
              autoComplete="username"
            />
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              placeholder={t("sync.password")}
              aria-label={t("sync.password")}
              autoComplete="current-password"
              onKeyDown={(e) => {
                if (e.key === "Enter" && !busy) void login();
              }}
            />
            <Input
              type="text"
              value={deviceName}
              onChange={(e) => setDeviceName(e.target.value)}
              placeholder={t("sync.deviceName")}
              aria-label={t("sync.deviceName")}
            />
            {advanced ? (
              <Input
                type="url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder={t("sync.hubUrlPlaceholder")}
                aria-label={t("sync.hubUrl")}
              />
            ) : (
              <button
                type="button"
                onClick={() => setAdvanced(true)}
                className="text-xs text-text/50 hover:text-text text-start cursor-pointer"
              >
                {t("sync.hubUrlToggle")}
              </button>
            )}
          </div>
          <Button
            onClick={() => void login()}
            disabled={busy || !email.trim() || !password}
          >
            <Cloud width={14} height={14} />
            {busy ? t("sync.loggingIn") : t("sync.login")}
          </Button>
        </div>
      ) : (
        <>
          <SettingContainer
            title={status?.user_email ?? ""}
            description={t("sync.accountDescription", {
              device: status?.device_name ?? "",
              hub: status?.hub_url ?? "",
            })}
            grouped={true}
            layout="horizontal"
          >
            <div className="flex items-center gap-2">
              <Button
                variant="secondary"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => void syncNow()}
                disabled={busy || status?.running}
              >
                <RefreshCw
                  width={14}
                  height={14}
                  className={status?.running ? "animate-spin" : ""}
                />
                {t("sync.syncNow")}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                onClick={() => setAbmelden(true)}
                disabled={busy}
              >
                <LogOut width={14} height={14} />
                {t("sync.logout")}
              </Button>
            </div>
          </SettingContainer>
          <div className="px-4 py-3 text-sm space-y-1" data-testid="sync-state">
            <p className="text-text/70">
              {status?.last_success_ms
                ? t("sync.lastSuccess", {
                    when: relativeTime(status.last_success_ms, i18n.language),
                  })
                : t("sync.neverSynced")}
              {" · "}
              {t("sync.pages", { count: status?.pages ?? 0 })}
              {status && status.pending > 0 && (
                <> · {t("sync.pending", { count: status.pending })}</>
              )}
            </p>
            {status?.last_error && (
              <p className="text-orange-500">{status.last_error}</p>
            )}
            {status?.key_mismatch && (
              <p className="text-orange-500">{t("sync.keyMismatch")}</p>
            )}
            {status && status.dead_letters > 0 && (
              <p className="text-orange-500">
                {t("sync.deadLetters", { count: status.dead_letters })}
              </p>
            )}
            {nurLokal > 0 && (
              <div
                className="flex flex-wrap items-center gap-2 pt-1"
                data-testid="sync-nur-lokal"
              >
                <span className="text-text/70">
                  {t("sync.nurLokal.anzahl", { count: nurLokal })}
                </span>
                <Button
                  variant="secondary"
                  size="sm"
                  className="max-sm:min-h-[44px] pointer-coarse:min-h-[44px]"
                  onClick={() => void nurLokalUebernehmen()}
                  disabled={busy || status?.running}
                  title={t("sync.nurLokal.hinweis")}
                >
                  <CloudUpload width={14} height={14} aria-hidden="true" />
                  {t("sync.nurLokal.uebernehmen")}
                </Button>
              </div>
            )}
            {hub && (hub.devices ?? []).length > 0 && (
              <p className="text-text/50">
                {t("sync.devices")}:{" "}
                {(hub.devices ?? [])
                  .map(
                    (d) =>
                      `${d.device.slice(0, 6)} (${d.last_push ? relativeTime(Date.parse(d.last_push + "Z"), i18n.language) : "–"})`,
                  )
                  .join(", ")}
              </p>
            )}
          </div>
        </>
      )}
      {hinweise}
      <SicherungenListe
        stand={`${sicherungStand}:${status?.connected}:${status?.last_success_ms}`}
      />
      <AnmeldungDatenDialog
        entscheidung={entscheidung}
        onErledigt={entschieden}
      />
      <AbmeldenDialog
        open={abmelden}
        konto={status?.user_email ?? ""}
        onOpenChange={setAbmelden}
        onAbgemeldet={abgemeldet}
        onFehler={() => {
          setSicherungStand((n) => n + 1);
          void refresh();
        }}
      />
    </SettingsGroup>
  );
};
