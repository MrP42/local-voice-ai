import React from "react";
import { useTranslation } from "react-i18next";
import type { ModelInfo } from "@/bindings";
import { LanguageSelector } from "../LanguageSelector";
import { TranslateToEnglish } from "../TranslateToEnglish";
import { StreamLookahead } from "../StreamLookahead";
import {
  CHINESE_LANGUAGE_CODE,
  getUniqueCapabilityLanguages,
} from "@/lib/constants/languages";

/**
 * Whether this model has anything to configure. Exported so a caller can
 * decide on the surrounding chrome (heading, separator) before rendering.
 */
export const modelHasOptions = (model: ModelInfo): boolean =>
  showsLanguageSelector(model) ||
  model.supports_translation ||
  model.supports_stream_lookahead;

const showsLanguageSelector = (model: ModelInfo): boolean => {
  if (model.supports_language_selection) return true;
  // Chinese-only models still offer a script choice (simplified/traditional),
  // which is a language selection in everything but name.
  const languages = getUniqueCapabilityLanguages(model.supported_languages);
  return languages.length === 1 && languages[0] === CHINESE_LANGUAGE_CODE;
};

/**
 * The options of one transcription model — currently its language and, where
 * supported, translation to English.
 *
 * These live on the model card rather than on a settings page. A language that
 * only exists because *this* model can detect it is a property of the model,
 * not of the app; sitting under "Settings → General" it read as a permanent
 * app option that mysteriously changed its heading whenever the active model
 * changed.
 *
 * The values themselves are global (`selected_language`,
 * `translate_to_english`) and apply to whichever model is in use — which is
 * why only the ACTIVE model shows them.
 */
export const ModelOptions: React.FC<{ model: ModelInfo }> = ({ model }) => {
  const { t } = useTranslation();

  if (!modelHasOptions(model)) return null;

  return (
    // Clicks inside must not bubble to the card, which would re-select the
    // model and reload the engine mid-edit.
    <div
      className="w-full mt-3 pt-3 border-t border-mid-gray/20"
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
    >
      <p className="text-xs font-medium uppercase tracking-wide text-text/50 mb-1">
        {t("settings.models.options.title")}
      </p>
      {showsLanguageSelector(model) && (
        <LanguageSelector
          descriptionMode="tooltip"
          grouped={true}
          supportedLanguages={model.supported_languages}
          supportsLanguageDetection={model.supports_language_detection}
          usesSystemLanguage={model.engine_type === "AppleSpeech"}
        />
      )}
      {model.supports_translation && (
        <TranslateToEnglish descriptionMode="tooltip" grouped={true} />
      )}
      {model.supports_stream_lookahead && <StreamLookahead />}
    </div>
  );
};
