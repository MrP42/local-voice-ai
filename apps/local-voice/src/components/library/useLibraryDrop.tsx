import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { BookCover } from "./BookCover";
import { extensionOf, kindOfFile } from "@/lib/library/books";
import {
  addToLibrary,
  findWork,
  freshBook,
  hasKind,
  prepareFile,
  type PreparedFile,
} from "@/lib/library/importFile";
import {
  audioProgressKey,
  loadProgress,
  progressFraction,
  textProgressKey,
} from "@/lib/library/progress";
import { useFileDrop } from "./useFileDrop";

type Choice = "attach" | "new" | "cancel";

/** Fortschritt der Fassung, die ein Ersetzen verwerfen würde (0–100). */
function progressOf(book: LibraryBook, kind: PreparedFile["kind"]): number {
  const key =
    kind === "text"
      ? book.text && textProgressKey(book.text.page_id)
      : book.audio && audioProgressKey(book.id);
  return key ? Math.round(progressFraction(loadProgress(key)) * 100) : 0;
}

interface Question {
  file: PreparedFile;
  book: LibraryBook;
  /** Das Buch hat diese Fassung schon: Anhängen ersetzt sie. */
  replaces: boolean;
  /** Abgelegt auf das Buch (sonst: als vorhandenes Werk erkannt). */
  dropped: boolean;
  resolve(choice: Choice): void;
}

/**
 * Drag & Drop in der Bibliothek (lva#152, Patrick 01.11.):
 * - auf freie Fläche: neues Buch (vorhandenes Werk → Rückfrage),
 * - auf ein Buch: passende Fassung anhängen; hat es die Fassung schon,
 *   Rückfrage „ersetzen oder als neues Buch“.
 * Mehrere Dateien werden nacheinander aufgenommen.
 */
export function useLibraryDrop(
  books: LibraryBook[] | null,
  onChanged: () => Promise<void>,
) {
  const { t } = useTranslation();
  const [question, setQuestion] = useState<Question | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const pending = useRef<((choice: Choice) => void) | null>(null);

  const gone = useRef(false);

  const ask = (q: Omit<Question, "resolve">) =>
    new Promise<Choice>((resolve) => {
      if (gone.current) return resolve("cancel");
      pending.current?.("cancel");
      pending.current = resolve;
      setQuestion({ ...q, resolve });
    });

  // Verlässt man die Bibliothek: offene Frage abbrechen und die restliche
  // Warteschlange leerlaufen lassen, statt an einer toten Frage zu hängen.
  useEffect(() => {
    gone.current = false;
    return () => {
      gone.current = true;
      pending.current?.("cancel");
    };
  }, []);

  const take = useCallback(
    async (path: string, targetId: string | null, asNew = false) => {
      if (gone.current) return;
      if (!kindOfFile(path)) {
        toast.error(t("library.unsupported", { ext: extensionOf(path) }));
        return;
      }
      setWorking(path.split(/[\\/]/).pop() ?? path);
      try {
        const file = await prepareFile(path);
        // Ziel frisch lesen: eine eben angehängte Fassung muss zählen.
        const target = targetId ? await freshBook(targetId) : null;
        let attachTo: LibraryBook | null = null;
        if (target) {
          // Abgelegt auf ein Buch: fehlt die Fassung, passt sie.
          if (!hasKind(target, file.kind)) attachTo = target;
          else {
            const choice = await ask({
              file,
              book: target,
              replaces: true,
              dropped: true,
            });
            if (choice === "cancel") return;
            attachTo = choice === "attach" ? target : null;
          }
        } else if (!asNew) {
          const work = await findWork(file.title, file.authors);
          if (work) {
            const choice = await ask({
              file,
              book: work,
              replaces: hasKind(work, file.kind),
              dropped: false,
            });
            if (choice === "cancel") return;
            attachTo = choice === "attach" ? work : null;
          }
        }
        const { book, attached } = await addToLibrary(file, {
          attachTo,
          onWarning: (m) => toast.error(m),
        });
        toast(
          attached
            ? t("library.attached", { title: book.title })
            : t("library.added", { title: book.title }),
        );
        if (!gone.current) await onChanged();
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        if (!gone.current) setWorking(null);
      }
    },
    [onChanged, t],
  );

  const queue = useRef(Promise.resolve());
  const drop = useFileDrop("[data-library-drop]", (paths, targetId) => {
    // Mehrere Dateien auf ein Buch: nur die erste je Art gilt diesem Buch,
    // weitere gleicher Art werden eigene Bücher (keine Ersetzen-Rückfrage
    // für das eben angehängte Buch).
    const used = new Set<string>();
    for (const path of paths) {
      const kind = kindOfFile(path) ?? "";
      const extra = !!targetId && used.has(kind);
      const target = targetId && !extra ? targetId : null;
      if (target) used.add(kind);
      queue.current = queue.current
        .then(() => take(path, target, extra))
        .catch(() => {});
    }
  });

  const answer = (choice: Choice) => {
    pending.current = null;
    question?.resolve(choice);
    setQuestion(null);
  };

  const kindLabel = question
    ? t(question.file.kind === "audio" ? "library.drop.kindAudio" : "library.drop.kindText")
    : "";
  const isAudio = question?.file.kind === "audio";
  const progress = question?.replaces ? progressOf(question.book, question.file.kind) : 0;
  const dialog = (
    <Dialog
      open={question !== null}
      onOpenChange={(open) => !open && answer("cancel")}
      title={
        question?.dropped
          ? t("library.drop.askTitleDropped", { title: question.book.title })
          : t("library.drop.askTitleKnown", { title: question?.book.title ?? "" })
      }
      closeLabel={t("library.cancel")}
      footer={
        <>
          <Button onClick={() => answer("new")} data-testid="library-drop-new">
            {t("library.drop.asNew")}
          </Button>
          <Button onClick={() => answer("cancel")}>{t("library.cancel")}</Button>
          <Button
            variant="primary"
            onClick={() => answer("attach")}
            data-testid="library-drop-attach"
          >
            {question?.replaces
              ? t(isAudio ? "library.drop.replaceAudio" : "library.drop.replaceText")
              : t("library.drop.attach")}
          </Button>
        </>
      }
    >
      {question && (
        <div className="library-add__merge" data-testid="library-drop-question">
          <BookCover book={question.book} size="xs" />
          <div className="library-drop-question__text">
            <strong style={{ overflowWrap: "anywhere" }}>{question.file.name}</strong>
            {question.replaces ? (
              <>
                <p data-testid="library-drop-consequence">
                  {t(
                    isAudio
                      ? "library.drop.replaceHintAudio"
                      : "library.drop.replaceHintText",
                    { title: question.book.title },
                  )}
                </p>
                {progress > 0 && (
                  <p data-testid="library-drop-progress">
                    <strong>
                      {t(
                        isAudio
                          ? "library.drop.progressAudio"
                          : "library.drop.progressText",
                        { pct: progress },
                      )}
                    </strong>
                  </p>
                )}
              </>
            ) : (
              <p>
                {t("library.drop.attachHint", {
                  kind: kindLabel,
                  title: question.book.title,
                })}
              </p>
            )}
            <p className="library-meta">
              {t("library.drop.newHint", { title: question.book.title })}
            </p>
          </div>
        </div>
      )}
    </Dialog>
  );

  return { drop, working, dialog };
}
