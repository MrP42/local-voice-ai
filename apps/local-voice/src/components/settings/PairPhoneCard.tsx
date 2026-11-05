import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { toast } from "sonner";
import { QrCode, AlertTriangle } from "lucide-react";
import { commands, type PairedDevice, type SyncStatus } from "@/bindings";
import { SettingsGroup } from "../ui/SettingsGroup";
import { SettingContainer } from "../ui/SettingContainer";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { relativeTime } from "./tts/WorkspaceSidebars";
import { PairPhoneDialog } from "./PairPhoneDialog";

/** Hub liefert `Y-m-d H:i:s` in seiner Ortszeit (Europe/Berlin, wie dieser PC). */
const hubMs = (s?: string | null) =>
  s ? Date.parse(s.replace(" ", "T")) : NaN;

/**
 * „Handy verbinden“ und „Verbundene Geräte“ (local-voice-ai#289, Spec Abschnitt 8),
 * unter der Sync-Karte. Nur sichtbar, wenn dieses Gerät am Hub angemeldet ist.
 */
export const PairPhoneCard: React.FC = () => {
  const { t, i18n } = useTranslation();
  const [connected, setConnected] = useState(false);
  const [devices, setDevices] = useState<PairedDevice[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [dialog, setDialog] = useState(false);
  const [revoke, setRevoke] = useState<PairedDevice | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    const r = await commands.syncDevices();
    if (r.status === "ok") {
      setDevices(r.data);
      setFailed(false);
    } else {
      setFailed(true);
    }
  }, []);

  useEffect(() => {
    void commands.syncStatus().then((s) => s && setConnected(s.connected));
    const unlisten = listen<SyncStatus>("sync-status", (e) =>
      setConnected(e.payload.connected),
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (connected) void load();
    else setDevices(null);
  }, [connected, load]);

  const reload = useCallback(() => void load(), [load]);

  if (!connected) return null;

  const byId = new Map((devices ?? []).map((d) => [d.id, d]));
  const date = (s?: string | null) => {
    const ms = hubMs(s);
    return Number.isNaN(ms)
      ? t("pairing.never")
      : new Date(ms).toLocaleDateString(i18n.language);
  };
  const ago = (s?: string | null) => {
    const ms = hubMs(s);
    return Number.isNaN(ms)
      ? t("pairing.never")
      : relativeTime(ms, i18n.language);
  };
  const origin = (d: PairedDevice) => {
    if (!d.via_pairing) return null;
    const parent = d.paired_by != null ? byId.get(d.paired_by) : undefined;
    return parent && !parent.current
      ? t("pairing.viaQrFrom", { name: parent.name })
      : t("pairing.viaQr");
  };

  const also = (revoke?.revokes_with ?? [])
    .map((id) => byId.get(id))
    .filter(Boolean) as PairedDevice[];
  const includesSelf = also.some((d) => d.current);

  const doRevoke = async () => {
    if (!revoke) return;
    setBusy(true);
    const r = await commands.syncRevokeDevice(revoke.id);
    setBusy(false);
    setRevoke(null);
    if (r.status === "ok") {
      toast.success(t("pairing.revoked"));
      void load();
      if (includesSelf) void commands.syncNow();
    } else {
      toast.error(t("pairing.revokeFailed"), { description: r.error });
    }
  };

  return (
    <>
      <SettingsGroup>
        <SettingContainer
          title={t("pairing.title")}
          description={t("pairing.cardDescription")}
          grouped={true}
          layout="horizontal"
        >
          <Button
            variant="primary"
            size="sm"
            className="min-h-11 sm:min-h-0"
            onClick={() => setDialog(true)}
            data-testid="pair-open"
          >
            <QrCode width={14} height={14} aria-hidden />
            {t("pairing.showQr")}
          </Button>
        </SettingContainer>
      </SettingsGroup>

      <SettingsGroup
        title={t("pairing.devicesTitle")}
        description={t("pairing.devicesDescription")}
      >
        {failed && (
          <p className="px-4 py-3 text-sm text-orange-500">
            {t("pairing.devicesFailed")}
          </p>
        )}
        {devices && (
          <div className="px-4 py-2 overflow-x-auto">
            <table className="w-full text-sm" data-testid="pair-devices">
              <thead className="text-left text-text/60">
                <tr>
                  <th scope="col" className="py-2 font-normal">
                    {t("pairing.colDevice")}
                  </th>
                  <th
                    scope="col"
                    className="py-2 font-normal hidden sm:table-cell"
                  >
                    {t("pairing.colSince")}
                  </th>
                  <th scope="col" className="py-2 font-normal">
                    {t("pairing.colActive")}
                  </th>
                  <th scope="col" className="py-2">
                    <span className="sr-only">{t("pairing.logout")}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                {devices.map((d) => (
                  <tr key={d.id} className="border-t border-mid-gray/20">
                    <td className="py-2 pr-3 min-w-[9rem]">
                      <div className="font-medium break-words">{d.name}</div>
                      {d.current && (
                        <div className="text-text/60 text-xs">
                          {t("pairing.thisDevice")}
                        </div>
                      )}
                      {origin(d) && (
                        <div className="text-text/60 text-xs">{origin(d)}</div>
                      )}
                    </td>
                    <td className="py-2 pr-3 whitespace-nowrap hidden sm:table-cell">
                      {date(d.created_at)}
                    </td>
                    <td className="py-2 pr-3 whitespace-nowrap">
                      {ago(d.last_used_at)}
                    </td>
                    <td className="py-2 text-right">
                      {!d.current && (
                        <Button
                          variant="danger-ghost"
                          size="sm"
                          className="min-h-11 sm:min-h-0"
                          onClick={() => setRevoke(d)}
                          aria-label={`${t("pairing.logout")}: ${d.name}`}
                        >
                          {t("pairing.logout")}
                        </Button>
                      )}
                    </td>
                  </tr>
                ))}
                {devices.length <= 1 && (
                  <tr>
                    <td colSpan={4} className="py-2 text-text/60">
                      {t("pairing.noDevices")}
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        )}
      </SettingsGroup>

      <PairPhoneDialog
        open={dialog}
        onOpenChange={setDialog}
        onChanged={reload}
      />

      <Dialog
        open={revoke !== null}
        onOpenChange={(o) => !o && setRevoke(null)}
        title={t("pairing.revokeTitle", { name: revoke?.name ?? "" })}
        closeLabel={t("pairing.close")}
        className="max-w-md"
        contentFades={false}
        footer={
          <>
            <Button variant="secondary" onClick={() => setRevoke(null)}>
              {t("pairing.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => void doRevoke()}
              disabled={busy}
              data-testid="pair-revoke-confirm"
            >
              {t("pairing.logout")}
            </Button>
          </>
        }
      >
        <div className="space-y-3 text-sm">
          {also.length > 0 && (
            <div className="flex gap-2" role="alert">
              <AlertTriangle
                className="shrink-0 text-orange-500"
                width={18}
                height={18}
                aria-hidden
              />
              <div>
                <p>{t("pairing.revokeAlso")}</p>
                <ul className="list-disc pl-5">
                  {also.map((d) => (
                    <li key={d.id} className="font-semibold text-text">
                      {d.name}
                    </li>
                  ))}
                </ul>
                {includesSelf && (
                  <p className="mt-1 font-medium">{t("pairing.revokeSelf")}</p>
                )}
              </div>
            </div>
          )}
          <p>{t("pairing.revokeText")}</p>
        </div>
      </Dialog>
    </>
  );
};
