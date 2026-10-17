import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  ErscheinungsbildEinstellungen,
  type ErscheinungsbildTexte,
} from "@/erscheinungsbild";

/**
 * Einstellungen → App → Erscheinungsbild: SDK-Standardabschnitt (Farbschema,
 * Kontrast, Animationen, Design-Karten, Akzent). Texte aus der App-i18n;
 * der Zustand kommt aus dem ErscheinungsbildProvider in FluentRoot.
 */
export const AppearanceSettings: React.FC = () => {
  const { t } = useTranslation();
  const texte = useMemo<ErscheinungsbildTexte>(
    () => ({
      titel: t("appearance.title"),
      farbschema: t("appearance.scheme"),
      farbschemaHilfe: t("appearance.schemeHelp"),
      modusSystem: t("theme.options.system"),
      modusHell: t("theme.options.light"),
      modusDunkel: t("theme.options.dark"),
      kontrast: t("appearance.contrast"),
      kontrastHilfe: t("appearance.contrastHelp"),
      kontrastStandard: t("appearance.contrastStandard"),
      kontrastHoch: t("appearance.contrastHigh"),
      animationen: t("appearance.animations"),
      animationenHilfe: t("appearance.animationsHelp"),
      animationenSystem: t("appearance.animationsSystem"),
      ein: t("appearance.on"),
      aus: t("appearance.off"),
      design: t("appearance.design"),
      designHilfe: t("appearance.designHelp"),
      akzent: t("appearance.accent"),
      akzentHilfe: t("appearance.accentHelp"),
      akzentHochKontrast: t("appearance.accentHighContrast"),
      akzentGewaehlt: t("appearance.accentSelected"),
      akzentGesperrt: t("appearance.accentLocked"),
      // SDK-Platzhalter {akzent}/{design} (einfache Klammern, i18next lässt sie stehen)
      akzentGruppe: t("appearance.accentGroup"),
      akzentNamen: t("appearance.accentNames", {
        returnObjects: true,
      }) as Record<string, string>,
      designs: {
        microsoft: {
          name: t("appearance.designNames.fluent"),
          beschreibung: t("appearance.designs.fluent"),
        },
        wai: {
          name: t("appearance.designNames.wai"),
          beschreibung: t("appearance.designs.wai"),
        },
      },
    }),
    [t],
  );
  return <ErscheinungsbildEinstellungen texte={texte} />;
};
