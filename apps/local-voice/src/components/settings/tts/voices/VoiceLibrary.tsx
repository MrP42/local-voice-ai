import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Dices, Save } from "lucide-react";
import { commands } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { SettingsGroup } from "@/components/ui/SettingsGroup";
import { SettingContainer } from "@/components/ui/SettingContainer";
import { Input } from "@/components/ui/Input";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { VoicesCard } from "../VoicesCard";
import { VoicePreviewButton } from "./VoicePreviewButton";

/**
 * Stimmen anhoeren und verwalten — auf der Modelle-Seite, nicht mehr unter
 * dem Vorlesen-Editor. Wer eine Stimme sucht, sucht kein Textfeld; und die
 * Vorlesen-Seite bleibt eine Arbeitsflaeche ohne Klappen darunter.
 *
 * Der Seed ist die Standardstimme: er entscheidet, WER spricht, und laesst
 * sich von hier aus als benannte Stimme sichern.
 */
export const VoiceLibrary = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const [saveSeedOpen, setSaveSeedOpen] = useState(false);
  const [seedName, setSeedName] = useState("");
  const [savingSeed, setSavingSeed] = useState(false);

  const saveSeedVoice = async () => {
    if (!seedName.trim()) return;
    setSavingSeed(true);
    const result = await commands.ttsSaveSeedVoice(seedName.trim());
    setSavingSeed(false);
    if (result.status === "error") {
      toast.error(result.error);
      return;
    }
    setSaveSeedOpen(false);
    setSeedName("");
    window.dispatchEvent(new CustomEvent("lv-voices-changed"));
    void updateSetting("tts_voice", result.data);
  };

  return (
    <div className="space-y-4" data-testid="voice-library">
      <SettingsGroup title={t("tts.settings.seed")}>
        <SettingContainer
          title={t("tts.settings.seed")}
          description={t("tts.settings.seedDescription")}
          grouped={true}
          layout="horizontal"
        >
          <div className="flex items-center gap-2">
            <Input
              type="number"
              value={getSetting("tts_seed") ?? 42}
              onChange={(e) => {
                const value = parseInt(e.target.value, 10);
                if (!isNaN(value)) updateSetting("tts_seed", value);
              }}
              disabled={isUpdating("tts_seed")}
              className="w-28"
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() =>
                updateSetting(
                  "tts_seed",
                  Math.floor(Math.random() * 2_147_483_647) + 1,
                )
              }
              disabled={isUpdating("tts_seed")}
            >
              <Dices width={14} height={14} />
              {t("tts.settings.rollSeed")}
            </Button>
            {/* Der Seed ist erst dann eine Wahl, wenn man ihn hoeren kann —
                direkt neben dem Wuerfel, mit dem man ihn aendert. Der
                Seed-Wert ist der Schluessel, damit nach dem Wuerfeln die
                neue Stimme erklingt und nicht die alte aus dem Cache. */}
            <VoicePreviewButton
              voiceId=""
              refreshKey={getSetting("tts_seed") ?? 42}
            />
            <Button
              variant="secondary"
              size="sm"
              onClick={() => setSaveSeedOpen(true)}
              disabled={savingSeed}
              title={t("tts.saveSeedHint")}
            >
              <Save width={14} height={14} />
              {t("tts.saveSeed")}
            </Button>
          </div>
        </SettingContainer>
      </SettingsGroup>

      <VoicesCard />

      <Dialog
        open={saveSeedOpen}
        onOpenChange={(open) => {
          setSaveSeedOpen(open);
          if (!open) setSeedName("");
        }}
        title={t("tts.saveSeedTitle")}
        closeLabel={t("tts.stopConfirmCancel")}
        footer={
          <>
            <Button variant="secondary" onClick={() => setSaveSeedOpen(false)}>
              {t("tts.stopConfirmCancel")}
            </Button>
            <Button
              onClick={saveSeedVoice}
              disabled={savingSeed || !seedName.trim()}
            >
              {savingSeed ? t("tts.saveSeedBusy") : t("tts.saveSeed")}
            </Button>
          </>
        }
      >
        <div className="space-y-2">
          <p className="text-sm text-text/80">{t("tts.saveSeedBody")}</p>
          <Input
            type="text"
            value={seedName}
            onChange={(e) => setSeedName(e.target.value)}
            placeholder={t("tts.saveSeedPlaceholder")}
            className="w-full"
          />
        </div>
      </Dialog>
    </div>
  );
};
