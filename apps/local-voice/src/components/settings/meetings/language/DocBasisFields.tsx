import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { DocBasis, TranscriptVariant } from "@/bindings";
import { Select } from "../../../ui/Select";
import { variantListLabel } from "../variants/useVariants";
import {
  SAME_AS_TRANSCRIPT,
  languageOptions,
  defaultOutputLanguage,
} from "./languages";

/** Die Wahl der beiden Felder: Fassung (`null` = aktive) und Ausgabesprache. */
export interface DocBasisChoice {
  variantId: string | null;
  /** Sprachcode oder `SAME_AS_TRANSCRIPT`. */
  outputLanguage: string;
}

/** Die Wahl als Argument der Befehle (`DocBasis`). */
export const toDocBasis = (choice: DocBasisChoice): DocBasis => ({
  variant_id: choice.variantId,
  output_language:
    choice.outputLanguage === SAME_AS_TRANSCRIPT ? null : choice.outputLanguage,
});

/**
 * Vorbelegung der Wahl: Grundlage = aktive Fassung, Ausgabesprache = zuletzt gewählte,
 * sonst die Sprache der App.
 */
export const initialDocBasisChoice = (
  i18nLanguage: string,
): DocBasisChoice => ({
  variantId: null,
  outputLanguage: defaultOutputLanguage(i18nLanguage) ?? SAME_AS_TRANSCRIPT,
});

/**
 * Das Argument für Läufe OHNE Dialog (Menü "Protokoll neu erzeugen", Knopf im Reiter):
 * aktive Fassung, Ausgabesprache wie die letzte Wahl bzw. die Sprache der App.
 */
export const defaultDocBasis = (i18nLanguage: string): DocBasis =>
  toDocBasis(initialDocBasisChoice(i18nLanguage));

interface DocBasisFieldsProps {
  variants: TranscriptVariant[];
  value: DocBasisChoice;
  onChange: (value: DocBasisChoice) => void;
}

/**
 * Zwei Auswahlfelder im Dialog "Neu erzeugen mit Vorlage": die Grundlage (welche Fassung
 * des Transkripts, Original oder Übersetzung) und die Sprache, in der das Dokument
 * geschrieben wird. Beides steht danach im Kopf des Protokolls und im Info-Dialog.
 */
export const DocBasisFields: React.FC<DocBasisFieldsProps> = ({
  variants,
  value,
  onChange,
}) => {
  const { t, i18n } = useTranslation();
  const active = variants.find((v) => v.active) ?? null;
  const variantOptions = useMemo(
    () =>
      variants.map((v) => {
        const label = variantListLabel(v, t);
        return {
          value: v.id,
          label: v.active
            ? t("meetings.variants.itemActive", { label })
            : label,
        };
      }),
    [variants, t],
  );
  const languageChoices = useMemo(
    () => [
      { value: SAME_AS_TRANSCRIPT, label: t("meetings.basis.sameAsBasis") },
      ...languageOptions(i18n.language),
    ],
    [i18n.language, t],
  );

  return (
    <div className="space-y-3" data-testid="docbasis-fields">
      {variants.length > 0 && (
        <div className="space-y-1">
          <span className="text-xs font-medium text-text/60">
            {t("meetings.basis.variant")}
          </span>
          <div data-testid="docbasis-variant">
            <Select
              ariaLabel={t("meetings.basis.variant")}
              value={value.variantId ?? active?.id ?? null}
              options={variantOptions}
              isClearable={false}
              menuPortal
              onChange={(id) => onChange({ ...value, variantId: id })}
            />
          </div>
        </div>
      )}
      <div className="space-y-1">
        <span className="text-xs font-medium text-text/60">
          {t("meetings.basis.outputLanguage")}
        </span>
        <div data-testid="docbasis-language">
          <Select
            ariaLabel={t("meetings.basis.outputLanguage")}
            value={value.outputLanguage}
            options={languageChoices}
            isClearable={false}
            menuPortal
            onChange={(code) =>
              onChange({ ...value, outputLanguage: code ?? SAME_AS_TRANSCRIPT })
            }
          />
        </div>
      </div>
      <p className="text-xs text-text/60">{t("meetings.basis.hint")}</p>
    </div>
  );
};
