import React from "react";
import { MeetingLanguageSetting } from "../meetings/MeetingLanguageSetting";
import { MeetingModelSetting } from "../meetings/MeetingModelSetting";
import { useTranslation } from "react-i18next";
import { type } from "@tauri-apps/plugin-os";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { ShortcutInput } from "../ShortcutInput";
import { PushToTalk } from "../PushToTalk";
import { VoiceActivityDetection } from "../VoiceActivityDetection";
import { CustomWords } from "../CustomWords";
import { AppendTrailingSpace } from "../AppendTrailingSpace";
import { PasteMethodSetting } from "../PasteMethod";
import { TypingToolSetting } from "../TypingTool";
import { ClipboardHandlingSetting } from "../ClipboardHandling";
import { AutoSubmit } from "../AutoSubmit";
import { useSettings } from "../../../hooks/useSettings";
import { DictationTest } from "../dictation-test/DictationTest";

/**
 * Everything about turning speech into text in another application, in the
 * order it happens: press a key, speak, get text inserted.
 *
 * The model's own options are deliberately absent — they live on the model
 * card under "Modelle" (see ModelOptions), because they belong to the model,
 * not to the app.
 *
 * The test bench closes the tab rather than occupying one of its own: you use
 * it right after changing something above it, and a tab would put a navigation
 * step between the change and the check.
 */
export const DictationTab: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const pushToTalk = getSetting("push_to_talk");
  const isLinux = type() === "linux";

  return (
    <div className="w-full space-y-6">
      <SettingsGroup title={t("settings.app.groups.shortcuts")}>
        <ShortcutInput shortcutId="transcribe" grouped={true} />
        <PushToTalk descriptionMode="tooltip" grouped={true} />
        {/* Hidden with push-to-talk (releasing the key cancels) and on Linux
            (dynamic shortcut instability) */}
        {!isLinux && !pushToTalk && (
          <ShortcutInput shortcutId="cancel" grouped={true} />
        )}
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.transcription")}>
        <VoiceActivityDetection descriptionMode="tooltip" grouped={true} />
        <CustomWords descriptionMode="tooltip" grouped />
        <AppendTrailingSpace descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("settings.advanced.groups.output")}>
        <PasteMethodSetting descriptionMode="tooltip" grouped={true} />
        <TypingToolSetting descriptionMode="tooltip" grouped={true} />
        <ClipboardHandlingSetting descriptionMode="tooltip" grouped={true} />
        <AutoSubmit descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>

      <SettingsGroup title={t("meetings.title")}>
        <MeetingLanguageSetting />
        <MeetingModelSetting />
      </SettingsGroup>
      <DictationTest />
    </div>
  );
};
