import { useTranslation } from "react-i18next";
import { ArrowRight, Play, X } from "lucide-react";
import type { BookTalkCitation } from "@/bindings";

/**
 * Belegkarte (lva#309): wörtliches Zitat, Kapitel, Seite; „Hinspringen“
 * markiert den Satz im Buch, „Ab hier vorlesen“ startet die Wiedergabe dort.
 */
export function CitationCard({
  citation,
  onJump,
  onReadFrom,
  onClose,
}: {
  citation: BookTalkCitation;
  onJump(c: BookTalkCitation): void;
  onReadFrom(c: BookTalkCitation): void;
  onClose(): void;
}) {
  const { t } = useTranslation();
  const where =
    citation.page != null
      ? t("tts.bookTalk.card.chapterPage", {
          chapter: citation.chapter_title,
          page: citation.page,
        })
      : t("tts.bookTalk.card.chapter", { chapter: citation.chapter_title });
  return (
    <section
      className="bt__card"
      aria-label={t("tts.bookTalk.card.region")}
      data-testid="booktalk-card"
    >
      <blockquote className="bt__quote" data-testid="booktalk-card-quote">
        {`„${citation.quote}“`}
      </blockquote>
      <p className="bt__where" data-testid="booktalk-card-where">
        {where}
      </p>
      <div className="bt__actions">
        <button
          type="button"
          onClick={() => onJump(citation)}
          data-testid="booktalk-jump"
        >
          <ArrowRight width={14} height={14} aria-hidden="true" />
          {t("tts.bookTalk.card.jump")}
        </button>
        <button
          type="button"
          onClick={() => onReadFrom(citation)}
          data-testid="booktalk-read-from"
        >
          <Play width={14} height={14} aria-hidden="true" />
          {t("tts.bookTalk.card.readFrom")}
        </button>
        <button
          type="button"
          aria-label={t("tts.bookTalk.card.close")}
          title={t("tts.bookTalk.card.close")}
          onClick={onClose}
        >
          <X width={14} height={14} aria-hidden="true" />
        </button>
      </div>
    </section>
  );
}
