import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/** Eingabe „a, b; c“ oder zeilenweise in einzelne Einträge. */
const splitDomains = (text: string): string[] =>
  text
    .split(/[\s,;]+/)
    .map((d) => d.trim())
    .filter((d) => d.length > 0);

/**
 * G9a-2 (lva#578, ADR-0031 E2.6–E2.8): Eingaben der Freigabe für KI-Clients unter dem
 * MCP-Schalter. „Eigene Domänen“ bestimmen, wer intern ist; „Cloud-Modelle in Besprechungen
 * mit Externen“ lockert nur bestätigtes „mit Externen“. Ein Vorschlag aus den eigenen
 * Adressen landet nur im Eingabefeld und wird erst mit „Speichern“ übernommen.
 */
export const MeetingAiReleaseSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating, refreshSettings } =
    useSettings();
  const stored = getSetting("meeting_own_domains") ?? [];
  const withExternals = getSetting("meeting_cloud_with_externals") ?? false;
  const [text, setText] = useState(() => stored.join(", "));
  const [note, setNote] = useState<string | null>(null);
  const [rejected, setRejected] = useState<string[]>([]);
  const [saving, setSaving] = useState(false);

  const storedKey = stored.join(", ");

  // Nur bei geänderter Einstellung neu füllen, nicht bei jeder Eingabe.
  useEffect(() => {
    setText(storedKey);
  }, [storedKey]);

  const save = async () => {
    setSaving(true);
    setNote(null);
    try {
      const result = await commands.changeMeetingOwnDomainsSetting(
        splitDomains(text),
      );
      if (result.status === "ok") {
        setRejected(result.data.abgelehnt);
        setText(result.data.gueltig.join(", "));
        setNote(t("meetings.mcp.release.ownDomainsSaved"));
        await refreshSettings();
      }
    } finally {
      setSaving(false);
    }
  };

  const suggest = async () => {
    const result = await commands.meetingOwnDomainsSuggestion();
    if (result.status !== "ok" || result.data.length === 0) {
      setNote(t("meetings.mcp.release.ownDomainsSuggestNone"));
      return;
    }
    const merged = Array.from(new Set([...splitDomains(text), ...result.data]));
    setText(merged.join(", "));
    setNote(t("meetings.mcp.release.ownDomainsSuggestHint"));
  };

  return (
    <>
      <SettingContainer
        title={t("meetings.mcp.release.ownDomains")}
        description={t("meetings.mcp.release.ownDomainsDescription")}
        grouped={true}
        layout="stacked"
        descriptionMode="tooltip"
      >
        <div className="space-y-2" data-testid="ai-release-settings">
          <p className="text-xs text-text/70" role="note">
            {t("meetings.mcp.release.description")}
          </p>
          <input
            type="text"
            className="w-full rounded-md border border-mid-gray/30 bg-background px-2 py-1 text-sm"
            value={text}
            placeholder={t("meetings.mcp.release.ownDomainsPlaceholder")}
            aria-label={t("meetings.mcp.release.ownDomains")}
            onChange={(e) => setText(e.target.value)}
            data-testid="own-domains-input"
          />
          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              onClick={() => void save()}
              disabled={saving}
              data-testid="own-domains-save"
            >
              {t("meetings.mcp.release.ownDomainsSave")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => void suggest()}
              data-testid="own-domains-suggest"
            >
              {t("meetings.mcp.release.ownDomainsSuggest")}
            </Button>
            <span
              className="text-xs text-text/70"
              role="status"
              data-testid="own-domains-status"
            >
              {note}
            </span>
          </div>
          {rejected.length > 0 && (
            <p
              className="text-xs text-amber-600 dark:text-amber-400"
              role="alert"
              data-testid="own-domains-rejected"
            >
              {t("meetings.mcp.release.ownDomainsRejected", {
                list: rejected.join(", "),
              })}
            </p>
          )}
        </div>
      </SettingContainer>
      <ToggleSwitch
        checked={withExternals}
        onChange={(v) => void updateSetting("meeting_cloud_with_externals", v)}
        isUpdating={isUpdating("meeting_cloud_with_externals")}
        label={t("meetings.mcp.release.cloudWithExternals")}
        description={t("meetings.mcp.release.cloudWithExternalsDescription")}
        descriptionMode="tooltip"
        grouped={true}
      />
    </>
  );
};
