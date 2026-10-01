import { useEffect, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { commands, type DocumentBasis } from "@/bindings";
import { baseLanguage, languageName } from "./languages";

export type BasisDocumentKind = "minutes" | "enhanced_notes";

/**
 * Die Zeile "Grundlage: Original (Englisch) · Protokoll auf Deutsch" aus den Angaben der
 * Dokumentversion. Dieselbe Zeile steht im Kopf der KI-Notizen und im Info-Dialog; im
 * Protokoll selbst steht sie schon im Text (das Backend schreibt sie in den Kopf).
 */
export const documentBasisText = (
  basis: DocumentBasis,
  kind: BasisDocumentKind,
  t: TFunction,
  locale: string,
): string => {
  const word = t(
    basis.variant_kind === "translation"
      ? "meetings.basis.translation"
      : "meetings.basis.original",
  );
  const sourceCode = baseLanguage(basis.language);
  const base = sourceCode
    ? t("meetings.basis.withLanguage", {
        variant: word,
        language: languageName(sourceCode, locale),
      })
    : word;
  const outputCode = baseLanguage(basis.output_language) ?? sourceCode;
  const documentKey = kind === "minutes" ? "Minutes" : "Notes";
  return outputCode
    ? t(`meetings.basis.line${documentKey}`, {
        basis: base,
        language: languageName(outputCode, locale),
      })
    : t(`meetings.basis.line${documentKey}Same`, { basis: base });
};

/**
 * Grundlage und Ausgabesprache der jüngsten Version von Protokoll oder KI-Notizen
 * (`null` ohne Dokument und bei Versionen aus der Zeit davor). Lädt neu, wenn sich
 * `refreshKey` ändert (neue Version).
 */
export function useDocumentBasis(
  meetingId: string,
  kind: BasisDocumentKind,
  /** Eine bestimmte Version; `null`: die jüngste. */
  documentId: string | null,
  refreshKey: string | number,
) {
  const [basis, setBasis] = useState<DocumentBasis | null>(null);
  const run = useRef(0);
  useEffect(() => {
    const seq = ++run.current;
    void commands
      .meetingsDocumentBasis(meetingId, kind, documentId)
      .then((result) => {
        if (seq !== run.current) return;
        setBasis(result.status === "ok" ? (result.data ?? null) : null);
      });
    return () => {
      run.current += 1;
    };
  }, [meetingId, kind, documentId, refreshKey]);
  return basis;
}
