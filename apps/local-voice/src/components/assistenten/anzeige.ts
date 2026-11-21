/**
 * Gemeinsame Anzeige-Bausteine der Seite „Assistenten“ (lva#556, G2 Teil 2): der Name einer Sprache in der Sprache der
 * Oberfläche und die Beschriftung eines Wertes aus den Übersetzungen.
 */
import { useMemo } from "react";
import { useTranslation } from "react-i18next";

/** Der Name zu einem Sprachcode der Profile (`auto` oder `xx` / `xx-XX`) in der Sprache der Oberfläche. */
export function useSprachName(): (code: string) => string {
  const { t, i18n } = useTranslation();
  return useMemo(() => {
    try {
      const namen = new Intl.DisplayNames([i18n.language], {
        type: "language",
      });
      return (code: string) =>
        code === "auto"
          ? t("assistenten.sprache.auto")
          : (namen.of(code) ?? code);
    } catch {
      return (code: string) =>
        code === "auto" ? t("assistenten.sprache.auto") : code;
    }
  }, [i18n.language, t]);
}
