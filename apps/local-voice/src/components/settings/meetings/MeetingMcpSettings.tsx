import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import {
  MCP_CLIENTS,
  mcpSnippet,
  type McpClient,
} from "../../../lib/mcpSnippets";
import { Button } from "../../ui/Button";
import { SettingContainer } from "../../ui/SettingContainer";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { useSettings } from "../../../hooks/useSettings";

/**
 * Lokaler MCP-Server (M6-P6e, F21) als Zeilen der Gruppe "Besprechungen": der
 * Schalter (Standard aus), bei "an" die Warnung, "Transkript freigeben" und der
 * kopierbare Konfig-Schnipsel fuer den KI-Client. Kein eigener Reiter. Der
 * Server liest die Schalter bei jedem Aufruf frisch, sie wirken ohne Neustart.
 */
export const MeetingMcpSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const enabled = getSetting("meeting_mcp_enabled") ?? false;
  const transcript = getSetting("meeting_mcp_include_transcript") ?? true;
  const [exePath, setExePath] = useState<string | null>(null);
  const [client, setClient] = useState<McpClient>("claude-code");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const resetTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands
      .meetingMcpInfo()
      .then((result) => {
        if (!cancelled && result.status === "ok") {
          setExePath(result.data?.exe_path ?? null);
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      if (resetTimer.current) clearTimeout(resetTimer.current);
    };
  }, []);

  const snippet = mcpSnippet(client, exePath);

  const copy = async () => {
    let next: "copied" | "failed" = "copied";
    try {
      await navigator.clipboard.writeText(snippet);
    } catch {
      next = "failed";
    }
    setCopyState(next);
    if (resetTimer.current) clearTimeout(resetTimer.current);
    resetTimer.current = setTimeout(() => setCopyState("idle"), 2500);
  };

  return (
    <>
      <ToggleSwitch
        checked={enabled}
        onChange={(v) => void updateSetting("meeting_mcp_enabled", v)}
        isUpdating={isUpdating("meeting_mcp_enabled")}
        label={t("meetings.mcp.title")}
        description={t("meetings.mcp.description")}
        descriptionMode="tooltip"
        grouped={true}
      />
      {enabled && (
        <>
          <ToggleSwitch
            checked={transcript}
            onChange={(v) =>
              void updateSetting("meeting_mcp_include_transcript", v)
            }
            isUpdating={isUpdating("meeting_mcp_include_transcript")}
            label={t("meetings.mcp.transcript")}
            description={t("meetings.mcp.transcriptDescription")}
            descriptionMode="tooltip"
            grouped={true}
          />
          <SettingContainer
            title={t("meetings.mcp.connect")}
            description={t("meetings.mcp.connectDescription")}
            grouped={true}
            layout="stacked"
            descriptionMode="tooltip"
          >
            <div className="space-y-2" data-testid="mcp-settings">
              <p
                className="text-sm text-amber-600 dark:text-amber-400"
                role="note"
                data-testid="mcp-warning"
              >
                {t("meetings.mcp.warning")}
              </p>
              <div
                className="flex flex-wrap gap-2"
                role="group"
                aria-label={t("meetings.mcp.connect")}
              >
                {MCP_CLIENTS.map((id) => (
                  <Button
                    key={id}
                    size="sm"
                    variant={id === client ? "primary" : "secondary"}
                    aria-pressed={id === client}
                    onClick={() => {
                      setClient(id);
                      setCopyState("idle");
                    }}
                    data-testid={`mcp-client-${id}`}
                  >
                    {t(`meetings.mcp.clients.${id}`)}
                  </Button>
                ))}
              </div>
              <pre
                className="whitespace-pre-wrap break-all rounded-lg border border-mid-gray/20 bg-mid-gray/10 px-3 py-2 text-xs"
                data-testid="mcp-snippet"
              >
                {snippet}
              </pre>
              <div className="flex items-center gap-3">
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void copy()}
                  data-testid="mcp-copy"
                >
                  {t("meetings.mcp.copy")}
                </Button>
                <span
                  className="text-xs text-text/70"
                  role="status"
                  data-testid="mcp-copy-status"
                >
                  {copyState === "copied" && t("meetings.mcp.copied")}
                  {copyState === "failed" && t("meetings.mcp.copyFailed")}
                </span>
              </div>
            </div>
          </SettingContainer>
        </>
      )}
    </>
  );
};
