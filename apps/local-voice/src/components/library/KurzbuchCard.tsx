import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, Play } from "lucide-react";
import { commands, type KiDocument, type LibraryBook } from "@/bindings";
import { Button } from "../ui/Button";
import { ProgressBar } from "./LibraryPage";
import {
  KURZBUCH_TEMPLATE_ID,
  LISTEN_WPM,
  READ_WPM,
  minutes,
  parseKurzbuch,
  type Kurzbuch,
} from "@/lib/kurzbuch/kurzbuch";
import { openKiDocInReader } from "@/lib/library/books";
import { listenedFraction } from "../settings/tts/kidocs/kiDocs";

/**
 * Kurzbuch im Buchdetail (lva#205): öffnen, weiterhören oder erstellen. Ein
 * Hörbuch ohne Text braucht erst sein Transkript.
 */
export function KurzbuchCard({
  book,
  transcribing,
  busy,
  onTranscribe,
}: {
  book: LibraryBook;
  transcribing: boolean;
  busy: boolean;
  onTranscribe: () => void;
}) {
  const { t } = useTranslation();
  const page = book.text?.page_id ?? null;
  const [found, setFound] = useState<{ doc: KiDocument; kb: Kurzbuch } | null>(
    null,
  );
  useEffect(() => {
    setFound(null);
    if (!page) return;
    let alive = true;
    void (async () => {
      const list = await commands.bookAiDocuments(page);
      if (list.status !== "ok" || !Array.isArray(list.data)) return;
      const doc = list.data
        .filter((d) => d.template_id === KURZBUCH_TEMPLATE_ID)
        .sort((a, b) => b.modified_ms - a.modified_ms)[0];
      if (!doc) return;
      const text = await commands.bookAiDocumentText(page, doc.id);
      if (!alive || text.status !== "ok") return;
      const kb = parseKurzbuch(text.data);
      if (kb) setFound({ doc, kb });
    })();
    return () => {
      alive = false;
    };
  }, [page]);

  if (!page && !book.audio_path) return null;
  const fraction = found && page ? listenedFraction(page, found.doc.id) : 0;

  return (
    <section className="library-kurzbuch" aria-labelledby="kurzbuch-head">
      <div className="library-kurzbuch__info">
        <h3 id="kurzbuch-head">{t("library.kurzbuch.title")}</h3>
        <span className="library-meta">
          {found
            ? t("library.kurzbuch.meta", {
                ideas: found.kb.ideas.length,
                read: minutes(found.kb.words, READ_WPM),
                listen: minutes(found.kb.words, LISTEN_WPM),
              }) +
              (fraction > 0
                ? ` · ${t("library.kurzbuch.heard", { percent: Math.round(fraction * 100) })}`
                : "")
            : page
              ? t("library.kurzbuch.none")
              : transcribing
                ? t("library.kurzbuch.transcribing")
                : t("library.kurzbuch.needsText")}
        </span>
        {found && fraction > 0 && (
          <ProgressBar
            fraction={fraction}
            label={t("library.kurzbuch.progress", { title: book.title })}
          />
        )}
      </div>
      <div className="library-kurzbuch__actions">
        {found && page ? (
          <>
            <Button
              icon={<BookOpen size={16} aria-hidden="true" />}
              onClick={() =>
                openKiDocInReader({ page, doc: found.doc.id, play: false })
              }
            >
              {t("library.kurzbuch.open")}
            </Button>
            <Button
              variant="primary"
              icon={<Play size={16} aria-hidden="true" />}
              onClick={() =>
                openKiDocInReader({ page, doc: found.doc.id, play: true })
              }
            >
              {fraction > 0
                ? t("library.kurzbuch.resume")
                : t("library.kurzbuch.listen")}
            </Button>
          </>
        ) : page ? (
          <Button
            onClick={() => openKiDocInReader({ page, create: "kurzbuch" })}
          >
            {t("library.kurzbuch.create")}
          </Button>
        ) : (
          !transcribing && (
            <Button onClick={onTranscribe} disabled={busy}>
              {t("library.transcribe")}
            </Button>
          )
        )}
      </div>
    </section>
  );
}
