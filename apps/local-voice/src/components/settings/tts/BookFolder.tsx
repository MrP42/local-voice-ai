import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  BookOpen,
  FileText,
  Headphones,
  Sparkles,
} from "lucide-react";
import {
  commands,
  type Edition,
  type KiDocument,
  type LibraryBook,
} from "@/bindings";
import { BookCover } from "../../library/BookCover";
import { Button } from "../../ui/Button";
import { formatDuration } from "@/lib/library/books";
import { editionsOf, langName, primaryEdition } from "@/lib/library/editions";
import { tracksOf } from "@/lib/library/names";

/**
 * Buch-Ordner im Vorlesen (lva#365 P1b): Kopf mit Cover und „Weiterlesen“,
 * dazu die Gruppen Fassungen, Hörbuch und KI-Dokumente. Die Fassungszeilen
 * selbst zeichnet die Seitenliste (Aktionen, Ansichten der offenen Seite);
 * hier stehen die Teile drumherum.
 */

/** Seiten-ID der Fassung, falls sie eine hat. */
const pageOf = (e: Edition): string | null => e.text?.page_id ?? null;

/** Sprache der Fassung zur Anzeige; ohne Angabe ein Platzhalter. */
export const editionLang = (
  e: Pick<Edition, "lang">,
  fallback: string,
): string => langName(e.lang) || fallback;

/** Seiten-ID → Beschriftung „Sprache · Hauptfassung“ für die Fassungszeilen. */
export function editionNotes(
  book: LibraryBook,
  mainLabel: string,
  unknown: string,
): Map<string, string> {
  const main = primaryEdition(book);
  const out = new Map<string, string>();
  for (const e of editionsOf(book)) {
    const page = pageOf(e);
    if (!page || out.has(page)) continue;
    const lang = editionLang(e, unknown);
    out.set(page, e.id === main?.id ? `${lang} · ${mainLabel}` : lang);
  }
  return out;
}

/** KI-Dokumente je Fassungsseite; lädt neu, wenn sich Seiten oder `refresh` ändern. */
export function useBookKiDocs(
  pageIds: string[],
  refresh: unknown,
): Map<string, KiDocument[]> {
  const key = pageIds.join("\n");
  const [docs, setDocs] = useState<Map<string, KiDocument[]>>(new Map());
  // Ein KI-Dokument einer anderen Fassung (Hintergrundlauf, Bibliothek) meldet
  // sich über `lv-kidocs-changed`: dann die Gruppe neu laden.
  const [changed, setChanged] = useState(0);
  useEffect(() => {
    const on = () => setChanged((n) => n + 1);
    window.addEventListener("lv-kidocs-changed", on);
    return () => window.removeEventListener("lv-kidocs-changed", on);
  }, []);
  useEffect(() => {
    let alive = true;
    const ids = key ? key.split("\n") : [];
    void Promise.all(
      ids.map(async (id) => {
        const r = await commands.bookAiDocuments(id).catch(() => null);
        const list =
          r && r.status === "ok" && Array.isArray(r.data) ? r.data : [];
        return [id, list] as const;
      }),
    ).then((entries) => {
      if (alive) setDocs(new Map(entries));
    });
    return () => {
      alive = false;
    };
  }, [key, refresh, changed]);
  return docs;
}

const GROUP_HEAD = "px-2 pt-3 pb-0.5 text-xs font-semibold text-text/60";
const ITEM =
  "reader-folders__item flex w-full min-h-[44px] items-center gap-2 rounded-md px-2 py-1 text-start transition-colors";
const ITEM_BUTTON = `${ITEM} cursor-pointer text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-logo-primary`;

/** Kopf des Buch-Ordners: Cover, Titel, Autoren, Anzahl der Fassungen, „Weiterlesen“. */
export const BookFolderHead: React.FC<{
  book: LibraryBook;
  onContinue: () => void;
}> = ({ book, onContinue }) => {
  const { t } = useTranslation();
  const count = editionsOf(book).length;
  return (
    <div
      className="book-folder-head tts-pages__list flex gap-3 border-b border-mid-gray/20 px-1 pb-3 pt-1"
      data-testid="book-folder-head"
    >
      <span className="book-folder-head__cover block w-14 shrink-0">
        <BookCover book={book} size="sm" />
      </span>
      <div className="book-folder-head__body min-w-0 flex-1">
        <h3 className="break-words text-sm font-semibold text-text">
          {book.title}
        </h3>
        <p className="break-words text-xs text-text/60">
          {[book.authors.join(", "), t("tts.folders.editions", { count })]
            .filter(Boolean)
            .join(" · ")}
        </p>
        <Button
          variant="primary"
          className="book-folder-head__continue reader-folders__btn mt-2"
          icon={<BookOpen width={16} height={16} aria-hidden="true" />}
          onClick={onContinue}
          data-testid="book-continue"
        >
          {t("tts.folders.continue")}
        </Button>
      </div>
    </div>
  );
};

/** Fassung, deren Quelle (Seite) nicht mehr auffindbar ist. Nichts wird gelöscht. */
export const MissingEdition: React.FC<{ edition: Edition }> = ({ edition }) => {
  const { t } = useTranslation();
  return (
    <div
      className={`tts-pages__list ${ITEM} text-text`}
      data-testid="edition-missing"
      data-edition={edition.id}
    >
      <FileText
        width={16}
        height={16}
        className="shrink-0 text-text/60"
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className="break-words text-sm">
            {editionLang(edition, t("tts.folders.unknownLang"))}
          </span>
          <span className="inline-flex items-center gap-1 rounded border border-orange-500/70 bg-orange-500/10 px-1.5 text-xs font-semibold text-text">
            <AlertTriangle
              width={12}
              height={12}
              className="text-orange-500"
              aria-hidden="true"
            />
            {t("tts.folders.missing")}
          </span>
        </span>
        <span className="block text-xs text-text/60">
          {t("tts.folders.missingHint")}
        </span>
      </span>
    </div>
  );
};

/** Hörbuch- und KI-Dokument-Gruppe unter den Fassungen. */
export const BookFolderGroups: React.FC<{
  book: LibraryBook;
  /** Seiten, die es gibt (die Seitenliste kennt sie). */
  pageIds: Set<string>;
  kiDocs: Map<string, KiDocument[]>;
  /** Geöffnetes KI-Dokument der offenen Seite. */
  activeDoc: { page: string; id: string } | null;
  onOpenPage: (pageId: string) => void;
  onOpenDoc: (pageId: string, docId: string) => void;
}> = ({ book, pageIds, kiDocs, activeDoc, onOpenPage, onOpenDoc }) => {
  const { t } = useTranslation();
  const unknown = t("tts.folders.unknownLang");
  const editions = useMemo(() => editionsOf(book), [book]);
  const main = primaryEdition(book);
  const audio = editions.filter((e) => e.audio);
  const aiRows = editions.flatMap((e) => {
    const page = pageOf(e);
    if (!page || !pageIds.has(page)) return [];
    return (kiDocs.get(page) ?? []).map((doc) => ({ edition: e, page, doc }));
  });
  return (
    <>
      {audio.length > 0 && (
        <section
          aria-label={t("tts.folders.groupAudio")}
          className="tts-pages__list"
        >
          <p className={GROUP_HEAD}>{t("tts.folders.groupAudio")}</p>
          {audio.map((e) => {
            const tracks = tracksOf(e.audio);
            const total = tracks.reduce(
              (sum, x) => sum + (x.duration_ms ?? 0),
              0,
            );
            const ms = total || (e.audio?.duration_ms ?? 0);
            const page = pageOf(e);
            const target = page && pageIds.has(page) ? page : null;
            const body = (
              <>
                <Headphones
                  width={16}
                  height={16}
                  className="shrink-0 text-text/60"
                  aria-hidden="true"
                />
                <span className="min-w-0 flex-1">
                  <span className="block break-words text-sm">
                    {t("tts.folders.audioTitle", {
                      lang: editionLang(e, unknown),
                    })}
                  </span>
                  <span className="block text-xs text-text/60">
                    {t("tts.folders.audioMeta", {
                      count: tracks.length,
                      duration: formatDuration(ms),
                    })}
                  </span>
                </span>
              </>
            );
            return target ? (
              <button
                key={e.id}
                type="button"
                data-testid="book-audio"
                className={ITEM_BUTTON}
                onClick={() => onOpenPage(target)}
              >
                {body}
              </button>
            ) : (
              <div
                key={e.id}
                data-testid="book-audio"
                className={`${ITEM} text-text`}
              >
                {body}
              </div>
            );
          })}
        </section>
      )}
      {aiRows.length > 0 && (
        <section
          aria-label={t("tts.folders.groupAi")}
          className="tts-pages__list"
        >
          <p className={GROUP_HEAD}>{t("tts.folders.groupAi")}</p>
          {aiRows.map(({ edition, page, doc }) => {
            const lang = editionLang(edition, unknown);
            const open = activeDoc?.page === page && activeDoc.id === doc.id;
            return (
              <button
                key={`${page}:${doc.id}`}
                type="button"
                data-testid="book-ai-doc"
                aria-current={open ? "true" : undefined}
                className={`${ITEM_BUTTON} ${open ? "bg-logo-primary/15 text-text" : ""}`}
                onClick={() => onOpenDoc(page, doc.id)}
              >
                <Sparkles
                  width={16}
                  height={16}
                  className="shrink-0 text-text/60"
                  aria-hidden="true"
                />
                <span className="min-w-0 flex-1">
                  <span className="block break-words text-sm">{doc.title}</span>
                  <span className="block text-xs text-text/60">
                    {edition.id === main?.id
                      ? t("tts.folders.aiMetaMain", { lang })
                      : t("tts.folders.aiMeta", { lang })}
                  </span>
                </span>
              </button>
            );
          })}
        </section>
      )}
    </>
  );
};

export { GROUP_HEAD };
