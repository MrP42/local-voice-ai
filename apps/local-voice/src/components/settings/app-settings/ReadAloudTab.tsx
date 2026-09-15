import React from "react";
import { useTtsAvailability } from "@/hooks/useTtsAvailability";
import { TtsModulesHint } from "../tts/TtsModulesHint";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";
import { Slider } from "../../ui/Slider";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { Input } from "../../ui/Input";
import { ShortcutInput } from "../ShortcutInput";
import { VoiceLibrary } from "../tts/voices/VoiceLibrary";
import { DEFAULT_TAG_PROVIDER_UI_VALUE } from "../tts/tags/AutoTagBar";

/// Erlaubte MP3-Bitraten (kbit/s) — dieselben vier Stufen wie in
/// `settings.rs`; mehr Stufen muesste die Oberflaeche auch erklaeren.
const EXPORT_BITRATES = [128, 192, 256, 320];

/**
 * Einstellungen des Vorlesens.
 *
 * Sie sassen bis hierher am Fuss der Vorlesen-Seite, in einer Klappe unter
 * dem Arbeitsbereich. Dort standen sie im Weg: wer vorliest, konfiguriert
 * nicht, und wer konfiguriert, sucht sie bei den Einstellungen. Was auf der
 * Vorlesen-Seite blieb, ist, was zur Arbeit gehoert — Stimmen, Seed und
 * Stimmwechsler.
 */
export const ReadAloudTab = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();

  const { fishInstalled, piperInstalled, systemVoices } = useTtsAvailability();

  return (
    <div className="w-full space-y-6">
    <TtsModulesHint />
    {(fishInstalled || piperInstalled || systemVoices.length > 0) && <SettingsGroup title={t("tts.settingsTitle")}>
      {/* Engine und Stimme werden auf der Vorlesen-Seite gewaehlt (Stimmen-
          Dropdown: Fish-Stimme oder "Name · Sprache · HQ · Piper"). Hier
          bleibt nur, was das Vorlesen dauerhaft einstellt. */}
{piperInstalled && <>
      <ToggleSwitch
        checked={getSetting("tts_piper_auto_language") ?? true}
        onChange={(checked) =>
          updateSetting("tts_piper_auto_language", checked)
        }
        isUpdating={isUpdating("tts_piper_auto_language")}
        label={t("tts.settings.piperAutoLanguage")}
        description={t("tts.settings.piperAutoLanguageDescription")}
        grouped={true}
      />
</>}
      {/* Auto-Tagging: Anbieter und Geraet sind Einstellungen, kein
          Arbeitsschritt -- auf der Vorlesen-Seite bleibt nur der Knopf. */}
{fishInstalled && <>
      <SettingContainer
        title={t("tts.settings.autotagProvider")}
        description={t("tts.settings.autotagProviderDescription")}
        grouped={true}
        layout="horizontal"
      >
        <div className="w-48">
          <Select
            value={
              (getSetting("tts_tag_provider") ?? "") === ""
                ? DEFAULT_TAG_PROVIDER_UI_VALUE
                : (getSetting("tts_tag_provider") ?? "")
            }
            options={[
              { value: DEFAULT_TAG_PROVIDER_UI_VALUE, label: t("tts.autotag.providerDefault") },
              { value: "anthropic", label: t("tts.autotag.providerClaude") },
            ]}
            isClearable={false}
            onChange={(value) =>
              updateSetting(
                "tts_tag_provider",
                value === DEFAULT_TAG_PROVIDER_UI_VALUE ? "" : (value ?? ""),
              )
            }
          />
        </div>
      </SettingContainer>
      <SettingContainer
        title={t("tts.settings.autotagDevice")}
        description={t("tts.autotag.deviceHint")}
        grouped={true}
        layout="horizontal"
      >
        <div className="w-48">
          <Select
            value={getSetting("tts_tag_device") ?? "auto"}
            options={[
              { value: "auto", label: t("tts.autotag.deviceAuto") },
              { value: "cpu", label: t("tts.autotag.deviceCpu") },
              { value: "gpu", label: t("tts.autotag.deviceGpu") },
            ]}
            isClearable={false}
            onChange={(value) => updateSetting("tts_tag_device", value ?? "auto")}
          />
        </div>
      </SettingContainer>
</>}
      <ShortcutInput shortcutId="speak_clipboard" grouped={true} />
      <Slider
        value={getSetting("tts_volume") ?? 1.0}
        onChange={(value) => updateSetting("tts_volume", value)}
        min={0}
        max={1}
        step={0.05}
        formatValue={(value) => `${Math.round(value * 100)}%`}
        label={t("tts.settings.volume")}
        description={t("tts.settings.volumeDescription")}
        grouped={true}
      />
      <ToggleSwitch
        checked={getSetting("tts_normalize") ?? true}
        onChange={(checked) => updateSetting("tts_normalize", checked)}
        isUpdating={isUpdating("tts_normalize")}
        label={t("tts.settings.normalize")}
        description={t("tts.settings.normalizeDescription")}
        grouped={true}
      />
{fishInstalled && <>
      <ToggleSwitch
        checked={getSetting("tts_prewarm") ?? false}
        onChange={(checked) => updateSetting("tts_prewarm", checked)}
        isUpdating={isUpdating("tts_prewarm")}
        label={t("tts.settings.prewarm")}
        description={t("tts.settings.prewarmDescription")}
        grouped={true}
      />
      <ToggleSwitch
        checked={getSetting("tts_reference_auto_transcribe") ?? true}
        onChange={(checked) =>
          updateSetting("tts_reference_auto_transcribe", checked)
        }
        isUpdating={isUpdating("tts_reference_auto_transcribe")}
        label={t("tts.settings.autoTranscribe")}
        description={t("tts.settings.autoTranscribeDescription")}
        grouped={true}
      />
      <ToggleSwitch
        checked={getSetting("tts_enhance") ?? true}
        onChange={(checked) => updateSetting("tts_enhance", checked)}
        isUpdating={isUpdating("tts_enhance")}
        label={t("tts.settings.enhance")}
        description={t("tts.settings.enhanceDescription")}
        grouped={true}
      />
      {(getSetting("tts_enhance") ?? true) && (
        <SettingContainer
          title={t("tts.settings.enhanceStrength")}
          description={t("tts.settings.enhanceStrengthDescription")}
          grouped={true}
          layout="horizontal"
        >
          <div className="w-40">
            <Select
              value={getSetting("tts_enhance_strength") ?? "gentle"}
              options={[
                {
                  value: "gentle",
                  label: t("tts.settings.strengthGentle"),
                },
                {
                  value: "medium",
                  label: t("tts.settings.strengthMedium"),
                },
                {
                  value: "strong",
                  label: t("tts.settings.strengthStrong"),
                },
              ]}
              onChange={(value) =>
                value &&
                updateSetting(
                  "tts_enhance_strength",
                  value as "gentle" | "medium" | "strong",
                )
              }
              isClearable={false}
            />
          </div>
        </SettingContainer>
      )}
</>}
      <SettingContainer
        title={t("tts.settings.exportFormat")}
        description={t("tts.settings.exportFormatDescription")}
        grouped={true}
        layout="horizontal"
      >
        <div className="w-36">
          {/* Formatnamen sind Eigennamen — bewusst nicht übersetzt. */}
          <Select
            value={getSetting("tts_export_format") ?? "wav"}
            options={[
              { value: "wav", label: "WAV" },
              { value: "mp3", label: "MP3" },
              { value: "opus", label: "Opus" },
            ]}
            isClearable={false}
            onChange={(value) => {
              if (value) updateSetting("tts_export_format", value);
            }}
          />
        </div>
      </SettingContainer>
      {/* Nur bei MP3: WAV kennt keine Bitrate, und Opus wird derzeit als
          WAV geschrieben. Eine sichtbare, aber wirkungslose Einstellung
          waere ein Versprechen, das der Export nicht haelt. */}
      {(getSetting("tts_export_format") ?? "wav") === "mp3" && (
        <SettingContainer
          title={t("tts.settings.exportBitrate")}
          description={t("tts.settings.exportBitrateDescription")}
          grouped={true}
          layout="horizontal"
        >
          <div className="w-36">
            <Select
              value={String(getSetting("tts_export_bitrate") ?? 192)}
              options={EXPORT_BITRATES.map((rate) => ({
                value: String(rate),
                label: t("tts.settings.exportBitrateOption", { rate }),
              }))}
              isClearable={false}
              onChange={(value) => {
                if (value) {
                  updateSetting("tts_export_bitrate", Number(value));
                }
              }}
            />
          </div>
        </SettingContainer>
      )}
{fishInstalled && <>
      <SettingContainer
        title={t("tts.settings.fishDir")}
        description={t("tts.settings.fishDirDescription")}
        grouped={true}
        layout="stacked"
      >
        <Input
          type="text"
          value={getSetting("tts_fish_dir") ?? ""}
          onChange={(e) => updateSetting("tts_fish_dir", e.target.value)}
          disabled={isUpdating("tts_fish_dir")}
          className="w-full"
        />
      </SettingContainer>
      <SettingContainer
        title={t("tts.settings.port")}
        description={t("tts.settings.portDescription")}
        grouped={true}
        layout="horizontal"
      >
        <Input
          type="number"
          min="1"
          max="65535"
          value={getSetting("tts_port") ?? 8080}
          onChange={(e) => {
            const value = parseInt(e.target.value, 10);
            if (!isNaN(value) && value > 0 && value <= 65535) {
              updateSetting("tts_port", value);
            }
          }}
          disabled={isUpdating("tts_port")}
          className="w-24"
        />
      </SettingContainer>
      <SettingContainer
        title={t("tts.settings.idleMinutes")}
        description={t("tts.settings.idleMinutesDescription")}
        grouped={true}
        layout="horizontal"
      >
        <Input
          type="number"
          min="0"
          max="1440"
          value={getSetting("tts_idle_minutes") ?? 15}
          onChange={(e) => {
            const value = parseInt(e.target.value, 10);
            if (!isNaN(value) && value >= 0) {
              updateSetting("tts_idle_minutes", value);
            }
          }}
          disabled={isUpdating("tts_idle_minutes")}
          className="w-24"
        />
      </SettingContainer>
      <ToggleSwitch
        checked={getSetting("tts_compile") ?? true}
        onChange={(checked) => updateSetting("tts_compile", checked)}
        isUpdating={isUpdating("tts_compile")}
        label={t("tts.settings.compile")}
        description={t("tts.settings.compileDescription")}
        grouped={true}
      />
</>}
      <ToggleSwitch
        checked={getSetting("tts_context_menu") ?? false}
        onChange={(checked) =>
          updateSetting("tts_context_menu", checked)
        }
        isUpdating={isUpdating("tts_context_menu")}
        label={t("tts.settings.contextMenu")}
        description={t("tts.settings.contextMenuDescription")}
        grouped={true}
      />
      <SettingContainer
        title={t("tts.settings.maxChars")}
        description={t("tts.settings.maxCharsDescription")}
        grouped={true}
        layout="horizontal"
      >
        <Input
          type="number"
          min="100"
          max="100000"
          value={getSetting("tts_max_chars") ?? 5000}
          onChange={(e) => {
            const value = parseInt(e.target.value, 10);
            if (!isNaN(value) && value >= 100) {
              updateSetting("tts_max_chars", value);
            }
          }}
          disabled={isUpdating("tts_max_chars")}
          className="w-24"
        />
      </SettingContainer>
    </SettingsGroup>}
    {/* Stimmen anhoeren, aufnehmen, klonen, importieren, loeschen: eine
        Einstellung des Vorlesens, deshalb hier -- nicht auf der Modelle-Seite
        (Entscheidung Patrick 14.09.). Die Vorlesen-Seite verweist ueber den
        Eintrag "Stimmen verwalten" im Stimmen-Dropdown hierher. */}
    {fishInstalled && <VoiceLibrary />}
    </div>
  );
};

export default ReadAloudTab;
