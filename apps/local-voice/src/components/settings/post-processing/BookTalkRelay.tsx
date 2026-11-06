import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  type HubDevice,
  type RelayGrantInfo,
  type RelayModelInfo,
  type RelayPairing,
} from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { ToggleSwitch } from "../../ui/ToggleSwitch";

/** Zwischenablage nach dem Kopieren des Codes nach dieser Zeit leeren. */
const CLIPBOARD_CLEAR_MS = 60_000;

/**
 * „iPhone darf Abo-Modelle nutzen“ (lva#320, Spec 11.3/11.7): Schalter (Standard aus),
 * Freigabe je iPhone mit gewählten Modellen (R1), Einrichtungscode (QR, Text als
 * Rückfall; 10 Minuten, einmalig) und Widerruf (R10). Der Relay-Schlüssel entsteht
 * am PC und verlässt ihn nur im Code; „Neuen Code erzeugen“ macht den alten ungültig.
 */
export const BookTalkRelay: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = (getSetting("book_talk_relay_enabled") ?? false) as boolean;
  const [models, setModels] = useState<RelayModelInfo[]>([]);
  const [grants, setGrants] = useState<RelayGrantInfo[]>([]);
  const [devices, setDevices] = useState<HubDevice[]>([]);
  const [codes, setCodes] = useState<Record<string, RelayPairing>>({});
  const [now, setNow] = useState(() => Date.now());
  const [names, setNames] = useState<Record<string, string>>({});
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [m, g, h] = await Promise.all([
        commands.booktalkRelayModels(),
        commands.booktalkRelayGrants(),
        commands.syncHubStatus(),
      ]);
      if (m.status === "ok") setModels(m.data);
      if (g.status === "ok") setGrants(g.data);
      if (h.status === "ok") setDevices(h.data.devices ?? []);
    } catch {
      // Ohne Backend: leere Listen.
    }
  }, []);

  useEffect(() => {
    if (enabled) void reload();
  }, [enabled, reload]);

  useEffect(() => {
    if (Object.keys(codes).length === 0) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [codes]);

  const copyCode = (code: string) => {
    void navigator.clipboard?.writeText(code);
    window.setTimeout(() => {
      void navigator.clipboard?.writeText("");
    }, CLIPBOARD_CLEAR_MS);
  };

  const grantOf = (key: string) => grants.find((g) => g.device_key === key);
  const chosen = (key: string) => picked[key] ?? models.map((m) => m.id);

  const authorize = async (key: string) => {
    setError(null);
    const res = await commands.booktalkRelayAuthorize(
      key,
      names[key] ?? "",
      chosen(key),
    );
    if (res.status === "ok") {
      setCodes((c) => ({ ...c, [key]: res.data }));
      await reload();
    } else {
      setError(String(res.error));
    }
  };

  const renewCode = async (key: string) => {
    const res = await commands.booktalkRelayRenewCode(key);
    if (res.status === "ok") {
      setCodes((c) => ({ ...c, [key]: res.data }));
      await reload();
    } else {
      setError(String(res.error));
    }
  };

  const revoke = async (key: string) => {
    await commands.booktalkRelayRevoke(key);
    setCodes((c) => {
      const { [key]: _gone, ...rest } = c;
      return rest;
    });
    await reload();
  };

  return (
    <div data-book-talk-relay>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => void updateSetting("book_talk_relay_enabled", v)}
        isUpdating={isUpdating("book_talk_relay_enabled")}
        label={t("settings.llm.functions.relay.title")}
        description={t("settings.llm.functions.relay.description")}
        descriptionMode="tooltip"
        grouped={true}
      />
      {enabled && (
        <div className="px-4 pb-3 space-y-3 text-sm">
          <p className="text-xs text-text/60">
            {t("settings.llm.functions.relay.quota")}
          </p>
          {models.length === 0 && (
            <p className="text-xs text-text/60" data-relay-no-models>
              {t("settings.llm.functions.relay.noModels")}
            </p>
          )}
          <p className="font-medium">
            {t("settings.llm.functions.relay.devices")}
          </p>
          {devices.length === 0 && (
            <p className="text-xs text-text/60">
              {t("settings.llm.functions.relay.noDevices")}
            </p>
          )}
          <ul className="space-y-3">
            {devices.map((d) => {
              const grant = grantOf(d.device);
              return (
                <li
                  key={d.device}
                  className="rounded border border-mid-gray/30 p-3 space-y-2"
                  data-relay-device={d.device}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-mono text-xs">
                      {grant ? grant.name : d.device}
                    </span>
                    {d.last_push && (
                      <span className="text-xs text-text/60">
                        {t("settings.llm.functions.relay.lastPush", {
                          when: d.last_push,
                        })}
                      </span>
                    )}
                  </div>
                  {grant ? (
                    <div className="space-y-2">
                      <p className="text-xs text-text/70">
                        {grant.pending
                          ? t("settings.llm.functions.relay.pendingNote", {
                              time: new Date(
                                grant.code_expires_at_ms,
                              ).toLocaleTimeString(),
                            })
                          : t("settings.llm.functions.relay.granted", {
                              date: new Date(
                                grant.expires_at_ms,
                              ).toLocaleDateString(),
                            })}{" "}
                        · {grant.model_ids.join(", ")}
                      </p>
                      <div className="flex gap-2">
                        <button
                          type="button"
                          className="px-2 py-1 rounded border border-mid-gray/40"
                          title={t("settings.llm.functions.relay.renewNote")}
                          onClick={() => void renewCode(d.device)}
                        >
                          {t("settings.llm.functions.relay.renew")}
                        </button>
                        <button
                          type="button"
                          className="px-2 py-1 rounded border border-mid-gray/40"
                          onClick={() => void revoke(d.device)}
                        >
                          {t("settings.llm.functions.relay.revoke")}
                        </button>
                      </div>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      <input
                        type="text"
                        className="w-full rounded border border-mid-gray/40 bg-transparent px-2 py-1"
                        aria-label={t("settings.llm.functions.relay.deviceName")}
                        placeholder={t(
                          "settings.llm.functions.relay.deviceName",
                        )}
                        value={names[d.device] ?? ""}
                        onChange={(e) =>
                          setNames((n) => ({
                            ...n,
                            [d.device]: e.target.value,
                          }))
                        }
                      />
                      <fieldset className="space-y-1">
                        <legend className="text-xs text-text/60">
                          {t("settings.llm.functions.relay.models")}
                        </legend>
                        {models.map((m) => (
                          <label
                            key={m.id}
                            className="flex items-center gap-2 text-xs"
                          >
                            <input
                              type="checkbox"
                              checked={chosen(d.device).includes(m.id)}
                              onChange={(e) =>
                                setPicked((p) => ({
                                  ...p,
                                  [d.device]: e.target.checked
                                    ? [...chosen(d.device), m.id]
                                    : chosen(d.device).filter(
                                        (id) => id !== m.id,
                                      ),
                                }))
                              }
                            />
                            {m.label}
                          </label>
                        ))}
                      </fieldset>
                      <button
                        type="button"
                        className="px-2 py-1 rounded border border-mid-gray/40 disabled:opacity-50"
                        disabled={
                          models.length === 0 || !(names[d.device] ?? "").trim()
                        }
                        onClick={() => void authorize(d.device)}
                      >
                        {t("settings.llm.functions.relay.authorize")}
                      </button>
                    </div>
                  )}
                  {codes[d.device] && (
                    <div className="space-y-2">
                      {codes[d.device].code_expires_at_ms <= now ? (
                        <p className="text-xs text-text/60" role="status">
                          {t("settings.llm.functions.relay.codeExpired")}
                        </p>
                      ) : (
                        <>
                          <p className="text-xs text-text/60">
                            {t("settings.llm.functions.relay.scan")}
                          </p>
                          <img
                            className="w-48 h-48 rounded bg-white"
                            alt={t("settings.llm.functions.relay.qrAlt")}
                            data-relay-qr
                            src={`data:image/svg+xml;utf8,${encodeURIComponent(
                              codes[d.device].qr_svg,
                            )}`}
                          />
                          <p className="text-xs text-text/60">
                            {t("settings.llm.functions.relay.codeFallback")}
                          </p>
                          <textarea
                            readOnly
                            rows={3}
                            className="w-full rounded border border-mid-gray/40 bg-transparent px-2 py-1 font-mono text-xs"
                            value={codes[d.device].code}
                            data-relay-code
                            onFocus={(e) => e.currentTarget.select()}
                          />
                          <button
                            type="button"
                            className="px-2 py-1 rounded border border-mid-gray/40"
                            onClick={() => copyCode(codes[d.device].code)}
                          >
                            {t("settings.llm.functions.relay.copy")}
                          </button>
                          <p className="text-xs text-text/60">
                            {t("settings.llm.functions.relay.clipboardNote")}
                          </p>
                        </>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
          {error && (
            <p className="text-xs text-red-500" role="alert">
              {t("settings.llm.functions.relay.error", { message: error })}
            </p>
          )}
        </div>
      )}
    </div>
  );
};

export default BookTalkRelay;
