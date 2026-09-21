import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/Button";
import { Dialog } from "@/components/ui/Dialog";
import { Input } from "@/components/ui/Input";

export interface ReplaceAllDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Der Text, in dem gesucht wird. */
  text: string;
  /** Vorbelegung des Suchfelds (Selektion oder Token unter dem Caret). */
  initialSearch: string;
  /** Wird mit dem fertigen Text aufgerufen (ein Undo-Schritt). */
  onReplace: (nextText: string, count: number) => void;
}

const escapeRegExp = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Alle Vorkommen zaehlen bzw. ersetzen — Wortgrenzen nur fuer reine
 *  Woerter; Marker `<…>` und Tags `[…]` tragen ihre Grenzen selbst. */
export function replaceAllOccurrences(
  text: string,
  search: string,
  replacement: string,
  caseSensitive: boolean,
  wholeWord: boolean,
): { text: string; count: number } {
  if (search === "") return { text, count: 0 };
  const isToken = /^[<[].*[>\]]$/.test(search);
  const body = escapeRegExp(search);
  const pattern =
    wholeWord && !isToken
      ? `(?<![\\p{L}\\p{N}_])${body}(?![\\p{L}\\p{N}_])`
      : body;
  const re = new RegExp(pattern, caseSensitive ? "gu" : "giu");
  let count = 0;
  const next = text.replace(re, () => {
    count++;
    return replacement;
  });
  return { text: next, count };
}

/**
 * Suchen und Ersetzen im ganzen Text — aus dem Kontextmenue heraus, mit der
 * Selektion vorbelegt. Unabhaengig von der Skript-Pruefung: auch ein
 * korrekter Sprecher oder ein gueltiges Tag darf ueberall getauscht werden.
 */
export const ReplaceAllDialog: React.FC<ReplaceAllDialogProps> = ({
  open,
  onOpenChange,
  text,
  initialSearch,
  onReplace,
}) => {
  const { t } = useTranslation();
  const [search, setSearch] = useState(initialSearch);
  const [replacement, setReplacement] = useState(initialSearch);
  const [caseSensitive, setCaseSensitive] = useState(true);
  const [wholeWord, setWholeWord] = useState(true);

  useEffect(() => {
    if (open) {
      setSearch(initialSearch);
      setReplacement(initialSearch);
    }
  }, [open, initialSearch]);

  const count = useMemo(
    () =>
      replaceAllOccurrences(text, search, replacement, caseSensitive, wholeWord)
        .count,
    [text, search, replacement, caseSensitive, wholeWord],
  );

  const run = () => {
    const result = replaceAllOccurrences(
      text,
      search,
      replacement,
      caseSensitive,
      wholeWord,
    );
    if (result.count > 0 && result.text !== text) {
      onReplace(result.text, result.count);
    }
    onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.replaceAll.title")}
      closeLabel={t("common.close")}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.stopConfirmCancel")}
          </Button>
          <Button
            onClick={run}
            disabled={count === 0 || search === replacement}
            data-testid="replace-all-run"
          >
            {t("tts.replaceAll.run", { count })}
          </Button>
        </>
      }
    >
      <div className="space-y-3 text-sm">
        <label className="block space-y-1">
          <span className="font-medium">{t("tts.replaceAll.search")}</span>
          <Input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full"
            data-testid="replace-all-search"
          />
        </label>
        <label className="block space-y-1">
          <span className="font-medium">{t("tts.replaceAll.replacement")}</span>
          <Input
            type="text"
            value={replacement}
            onChange={(e) => setReplacement(e.target.value)}
            className="w-full"
            autoFocus
            data-testid="replace-all-replacement"
            onKeyDown={(e) => {
              if (e.key === "Enter") run();
            }}
          />
        </label>
        <div className="flex flex-wrap gap-4 text-xs text-text/80">
          <label className="flex items-center gap-1.5 cursor-pointer">
            <input
              type="checkbox"
              checked={caseSensitive}
              onChange={(e) => setCaseSensitive(e.target.checked)}
            />
            {t("tts.replaceAll.caseSensitive")}
          </label>
          <label className="flex items-center gap-1.5 cursor-pointer">
            <input
              type="checkbox"
              checked={wholeWord}
              onChange={(e) => setWholeWord(e.target.checked)}
            />
            {t("tts.replaceAll.wholeWord")}
          </label>
        </div>
        <p className="text-xs text-text/60" data-testid="replace-all-count">
          {t("tts.replaceAll.matches", { count })}
        </p>
      </div>
    </Dialog>
  );
};
