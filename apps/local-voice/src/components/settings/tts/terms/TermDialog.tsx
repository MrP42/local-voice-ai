import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BookOpenText,
  ExternalLink,
  NotebookPen,
  Play,
  Volume2,
  VolumeX,
} from "lucide-react";
import type { BookTerm } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import {
  SEARCH_PRESETS,
  validSearchUrl,
  type SearchPreset,
} from "./termSearch";
import "./terms.css";

/**
 * Erklärfenster eines Begriffs (lva#185). Das Vorlesen ist beim Öffnen schon
 * angehalten; die Erklärung wird sofort gesprochen (abschaltbar). Was das Buch
 * sagt und was allgemeines Modellwissen ist, steht getrennt da.
 */
export function TermDialog({
  term,
  page,
  open,
  onOpenChange,
  autoRead,
  onAutoRead,
  speaking,
  onSpeak,
  onStopSpeaking,
  onContinue,
  canContinue,
  onMore,
  preset,
  onPreset,
  customUrl,
  onCustomUrl,
}: {
  term: BookTerm | null;
  /** Seitenmarke der Fundstelle (Originalseite), falls bekannt. */
  page: number | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  autoRead: boolean;
  onAutoRead: (on: boolean) => void;
  /** Die Erklärung wird gerade vorgelesen. */
  speaking: boolean;
  onSpeak: () => void;
  onStopSpeaking: () => void;
  /** Am unterbrochenen Satz weiterlesen. */
  onContinue: () => void;
  /** Lief das Vorlesen beim Öffnen? Sonst heißt der Knopf „Hier vorlesen“. */
  canContinue: boolean;
  onMore: () => void;
  preset: SearchPreset;
  onPreset: (p: SearchPreset) => void;
  customUrl: string;
  onCustomUrl: (url: string) => void;
}) {
  const { t } = useTranslation();
  const continueRef = useRef<HTMLButtonElement>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  if (!term) return null;
  const where = [
    term.chapter.trim(),
    page !== null ? t("tts.terms.page", { page }) : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const customBad = preset === "custom" && !validSearchUrl(customUrl);
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={term.term}
      closeLabel={t("tts.terms.close")}
      className="term-dialog max-w-xl"
      initialFocusRef={continueRef as React.RefObject<HTMLElement>}
      footer={
        <>
          <Button
            ref={continueRef}
            variant="primary"
            icon={<Play className="h-4 w-4" aria-hidden="true" />}
            onClick={onContinue}
            data-testid="term-continue"
          >
            {canContinue ? t("tts.terms.continue") : t("tts.terms.readHere")}
          </Button>
          <Button
            variant="secondary"
            icon={<ExternalLink className="h-4 w-4" aria-hidden="true" />}
            onClick={onMore}
            disabled={customBad}
            data-testid="term-more"
          >
            {t("tts.terms.more")}
          </Button>
          <Button
            variant="secondary"
            icon={<NotebookPen className="h-4 w-4" aria-hidden="true" />}
            disabled
            title={t("tts.terms.toNotesSoon")}
            aria-describedby="term-notes-soon"
            data-testid="term-notes"
          >
            {t("tts.terms.toNotes")}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3" data-testid="term-dialog">
        <p className="term-dialog__meta">
          <span className="term-dialog__kind">
            {t(`tts.terms.kind.${term.kind}`)}
          </span>
          {where && <span>{where}</span>}
        </p>
        <section aria-labelledby="term-book-h">
          <h3 id="term-book-h" className="term-dialog__h">
            <BookOpenText className="h-4 w-4" aria-hidden="true" />
            {t("tts.terms.fromBook")}
          </h3>
          <p className="text-sm" data-testid="term-from-book">
            {term.from_book.trim() || t("tts.terms.fromBookNone")}
          </p>
        </section>
        {term.general.trim() && (
          <section
            aria-labelledby="term-general-h"
            className="term-dialog__general"
          >
            <h3 id="term-general-h" className="term-dialog__h">
              {t("tts.terms.general")}
            </h3>
            <p className="text-sm" data-testid="term-general">
              {term.general}
            </p>
            <p className="term-dialog__note">{t("tts.terms.generalNote")}</p>
          </section>
        )}
        <div className="term-dialog__speak">
          <Button
            variant="ghost"
            size="sm"
            icon={
              speaking ? (
                <VolumeX className="h-4 w-4" aria-hidden="true" />
              ) : (
                <Volume2 className="h-4 w-4" aria-hidden="true" />
              )
            }
            onClick={speaking ? onStopSpeaking : onSpeak}
            data-testid="term-speak"
          >
            {speaking ? t("tts.terms.stopSpeaking") : t("tts.terms.speak")}
          </Button>
          <label className="term-dialog__check">
            <input
              type="checkbox"
              checked={autoRead}
              onChange={(e) => onAutoRead(e.target.checked)}
              data-testid="term-autoread"
            />
            {t("tts.terms.autoRead")}
          </label>
        </div>
        <p id="term-notes-soon" className="term-dialog__note">
          {t("tts.terms.toNotesSoon")}
        </p>
        <details
          className="term-dialog__search"
          open={searchOpen}
          onToggle={(e) => setSearchOpen((e.target as HTMLDetailsElement).open)}
        >
          <summary data-testid="term-search-toggle">
            {t("tts.terms.searchSettings")}
          </summary>
          <div
            role="radiogroup"
            aria-label={t("tts.terms.searchSettings")}
            className="mt-2 flex flex-col gap-1"
          >
            {(Object.keys(SEARCH_PRESETS) as SearchPreset[])
              .concat("custom")
              .map((p) => (
                <label key={p} className="term-dialog__check">
                  <input
                    type="radio"
                    name="term-search"
                    checked={preset === p}
                    onChange={() => onPreset(p)}
                    data-testid={`term-search-${p}`}
                  />
                  {t(`tts.terms.search.${p}`)}
                </label>
              ))}
            {preset === "custom" && (
              <>
                <Input
                  value={customUrl}
                  onChange={(e) => onCustomUrl(e.target.value)}
                  placeholder="https://example.org/suche?q={q}"
                  aria-label={t("tts.terms.search.customLabel")}
                  aria-invalid={customBad}
                  aria-describedby="term-search-hint"
                  data-testid="term-search-url"
                />
                <p id="term-search-hint" className="term-dialog__note">
                  {t("tts.terms.search.customHint")}
                </p>
              </>
            )}
          </div>
        </details>
      </div>
    </Dialog>
  );
}
