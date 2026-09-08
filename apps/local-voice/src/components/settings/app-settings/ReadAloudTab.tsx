import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands } from "@/bindings";
import { useSettings } from "../../../hooks/useSettings";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { SettingContainer } from "../../ui/SettingContainer";
import { Select } from "../../ui/Select";
import { Slider } from "../../ui/Slider";
import { ToggleSwitch } from "../../ui/ToggleSwitch";
import { Input } from "../../ui/Input";
import { ShortcutInput } from "../ShortcutInput";

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

  // Heruntergeladene Piper-Stimmen. Nur sie kann Piper vorlesen; die Liste
  // kommt aus demselben Download-Verzeichnis, das die Modellseite fuellt.
  const [piperVoices, setPiperVoices] = useState<
    { id: string; name: string }[]
  >([]);
  useEffect(() => {
    void commands.ttsListDownloads().then((result) => {
      if (result.status !== "ok") return;
      // Faellt die Abfrage aus, bleibt die Liste leer statt undefiniert.
      setPiperVoices(
        (result.data ?? [])
          .filter((entry) => entry.kind === "voice" && entry.is_downloaded)
          .map((entry) => ({ id: entry.id, name: entry.name })),
      );
    });
  }, []);

  return (
    <div className="w-full space-y-6">
    <SettingsGroup title={t("tts.settingsTitle")}>
      {/* Die Wahl der Engine stand bisher nur in den Einstellungen
          auf der Platte, nicht in der Oberflaeche: Piper liess sich
          herunterladen, aber nicht einschalten. Sie gehoert an den
          Anfang — sie entscheidet, was alle Regler darunter tun. */}
      <SettingContainer
        title={t("tts.settings.engine")}
        description={t("tts.settings.engineDescription")}
        grouped={true}
        layout="horizontal"
      >
        <div className="w-48">
          <Select
            value={getSetting("tts_engine") ?? "fish"}
            options={[
              { value: "fish", label: t("tts.settings.engineFish") },
              { value: "piper", label: t("tts.settings.enginePiper") },
            ]}
            isClearable={false}
            onChange={(value) => {
              if (value) updateSetting("tts_engine", value);
            }}
          />
        </div>
      </SettingContainer>
      {(getSetting("tts_engine") ?? "fish") === "piper" &&
        (piperVoices.length > 0 ? (
          <SettingContainer
            title={t("tts.settings.piperVoice")}
            description={t("tts.settings.piperVoiceDescription")}
            grouped={true}
            layout="horizontal"
          >
            <div className="w-48">
              <Select
                value={getSetting("tts_piper_voice") ?? ""}
                options={piperVoices.map((voice) => ({
                  value: voice.id,
                  label: voice.name,
                }))}
                isClearable={false}
                onChange={(value) => {
                  if (value) updateSetting("tts_piper_voice", value);
                }}
              />
            </div>
          </SettingContainer>
        ) : (
          /* Piper ohne Stimme kann nichts vorlesen. Der Hinweis sagt,
             wo sie herkommt, statt die Auswahl leer zu zeigen. */
          <SettingContainer
            title={t("tts.settings.piperVoice")}
            description={t("tts.settings.piperVoiceMissing")}
            grouped={true}
            layout="horizontal"
          >
            <span />
          </SettingContainer>
        ))}
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
    </SettingsGroup>
    </div>
  );
};

export default ReadAloudTab;
