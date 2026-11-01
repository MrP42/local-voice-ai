import React, { useCallback, useRef, useState } from "react";
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
  hasKind,
  prepareFile,
  type PreparedFile,
} from "@/lib/library/importFile";
import { useFileDrop } from "./useFileDrop";

type Choice = "attach" | "new" | "cancel";

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
  const booksRef = useRef(books);
  booksRef.current = books;

  const ask = (q: Omit<Question, "resolve">) =>
    new Promise<Choice>((resolve) => setQuestion({ ...q, resolve }));

  const take = useCallback(
    async (path: string, targetId: string | null) => {
      if (!kindOfFile(path)) {
        toast.error(t("library.unsupported", { ext: extensionOf(path) }));
        return;
      }
      setWorking(path.split(/[\\/]/).pop() ?? path);
      try {
        const file = await prepareFile(path);
        const target = targetId
          ? (booksRef.current ?? []).find((b) => b.id === targetId) ?? null
          : null;
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
        } else {
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
        await onChanged();
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        setWorking(null);
      }
    },
    [onChanged, t],
  );

  const queue = useRef(Promise.resolve());
  const drop = useFileDrop("[data-library-drop]", (paths, targetId) => {
    for (const path of paths)
      queue.current = queue.current.then(() => take(path, targetId));
  });

  const answer = (choice: Choice) => {
    question?.resolve(choice);
    setQuestion(null);
  };

  const kindLabel = question
    ? t(question.file.kind === "audio" ? "library.drop.kindAudio" : "library.drop.kindText")
    : "";
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
            {question?.replaces ? t("library.drop.replace") : t("library.drop.attach")}
          </Button>
        </>
      }
    >
      {question && (
        <div className="library-add__merge" data-testid="library-drop-question">
          <BookCover book={question.book} size="xs" />
          <div>
            <strong style={{ overflowWrap: "anywhere" }}>{question.file.name}</strong>
            <div className="library-meta">
              {question.replaces
                ? t("library.drop.replaceHint", {
                    kind: kindLabel,
                    title: question.book.title,
                  })
                : t("library.drop.attachHint", {
                    kind: kindLabel,
                    title: question.book.title,
                  })}
            </div>
          </div>
        </div>
      )}
    </Dialog>
  );

  return { drop, working, dialog };
}
