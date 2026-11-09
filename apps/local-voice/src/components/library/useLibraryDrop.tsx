import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { BookCover } from "./BookCover";
import { extensionOf, fileNameOf, kindOfFile } from "@/lib/library/books";
import { groupAudioFiles, type AudioGroup } from "@/lib/library/audioGroups";
import {
  addAudioGroup,
  addToLibrary,
  drmMessage,
  findWork,
  freshBook,
  hasKind,
  prepareFile,
  type PreparedFile,
} from "@/lib/library/importFile";
import {
  loadProgress,
  progressFraction,
  textProgressKey,
} from "@/lib/library/progress";
import { primaryAudioKey } from "@/lib/library/editions";
import { useFileDrop } from "./useFileDrop";

type Choice = "attach" | "append" | "new" | "cancel";

/** Fortschritt der Fassung, die ein Ersetzen verwerfen würde (0–100). */
function progressOf(book: LibraryBook, kind: PreparedFile["kind"]): number {
  const key =
    kind === "text"
      ? book.text && textProgressKey(book.text.page_id)
      : book.audio && primaryAudioKey(book);
  return key ? Math.round(progressFraction(loadProgress(key)) * 100) : 0;
}

interface Question {
  /** Anzeigename: Dateiname oder „36 Dateien“. */
  name: string;
  kind: PreparedFile["kind"];
  /** Zahl der abgelegten Dateien dieser Frage. */
  count: number;
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
 *   Rückfrage „ersetzen, als weitere Teile anhängen oder als neues Buch“.
 * Audiodateien werden nach Ordner und Titelstamm gruppiert (lva#204): eine
 * CD-Box mit 36 Dateien ist **ein** Buch. Auf ein Buch abgelegt werden alle
 * Audiodateien Teile seiner Audiofassung. Textdateien nacheinander.
 */
export function useLibraryDrop(
  books: LibraryBook[] | null,
  onChanged: () => Promise<void>,
  /** Neu aufgenommenes Buch: als Sprachfassung eines vorhandenen anbieten (lva#344). */
  offerFor?: (book: LibraryBook) => Promise<string>,
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
      setWorking(fileNameOf(path));
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
              name: file.name,
              kind: file.kind,
              count: 1,
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
              name: file.name,
              kind: file.kind,
              count: 1,
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
        if (!attached && !gone.current) await offerFor?.(book);
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        if (!gone.current) setWorking(null);
      }
    },
    [onChanged, offerFor, t],
  );

  /** Eine Gruppe Audiodateien als ein Hörbuch aufnehmen. */
  const takeAudio = useCallback(
    async (group: AudioGroup, targetId: string | null) => {
      if (gone.current) return;
      setWorking(
        group.paths.length > 1
          ? t("library.drop.files", { count: group.paths.length, name: group.title })
          : fileNameOf(group.paths[0]),
      );
      try {
        const target = targetId ? await freshBook(targetId) : null;
        const work = target
          ? null
          : await findWork(group.title, group.authors);
        const book = target ?? work;
        let attachTo: LibraryBook | null = null;
        let append = false;
        if (book) {
          if (target && !target.audio) attachTo = target;
          else {
            const choice = await ask({
              name:
                group.paths.length > 1
                  ? t("library.drop.files", { count: group.paths.length, name: group.title })
                  : fileNameOf(group.paths[0]),
              kind: "audio",
              count: group.paths.length,
              book,
              replaces: !!book.audio,
              dropped: !!target,
            });
            if (choice === "cancel") return;
            if (choice !== "new") {
              attachTo = book;
              append = choice === "append";
            }
          }
        }
        const { book: saved, attached } = await addAudioGroup(group, {
          attachTo,
          append,
          onWarning: (m) => toast.error(m),
        });
        toast(
          group.paths.length > 1
            ? t(attached ? "library.attachedParts" : "library.addedParts", {
                title: saved.title,
                count: group.paths.length,
              })
            : attached
              ? t("library.attached", { title: saved.title })
              : t("library.added", { title: saved.title }),
        );
        if (!gone.current) await onChanged();
        if (!attached && !gone.current) await offerFor?.(saved);
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        if (!gone.current) setWorking(null);
      }
    },
    [onChanged, offerFor, t],
  );

  const queue = useRef(Promise.resolve());
  const handlePaths = (paths: string[], targetId: string | null) => {
    const texts: string[] = [];
    const audios: string[] = [];
    for (const path of paths) {
      const kind = kindOfFile(path);
      if (kind === "text") texts.push(path);
      else if (kind === "audio") audios.push(path);
      else if (kind === "drm") toast.error(drmMessage(fileNameOf(path)));
      else toast.error(t("library.unsupported", { ext: extensionOf(path) }));
    }
    // Mehrere Texte auf ein Buch: nur der erste gilt diesem Buch, weitere
    // werden eigene Bücher (keine Ersetzen-Rückfrage für das eben angehängte).
    texts.forEach((path, i) => {
      const target = targetId && i === 0 ? targetId : null;
      queue.current = queue.current
        .then(() => take(path, target, !!targetId && i > 0))
        .catch(() => {});
    });
    if (!audios.length) return;
    if (targetId) {
      // Auf ein Buch: alle Audiodateien sind Teile seiner Audiofassung.
      const group: AudioGroup = {
        ...groupAudioFiles(audios)[0],
        paths: groupAudioFiles(audios).flatMap((g) => g.paths),
      };
      queue.current = queue.current
        .then(() => takeAudio(group, targetId))
        .catch(() => {});
      return;
    }
    for (const group of groupAudioFiles(audios))
      queue.current = queue.current
        .then(() => takeAudio(group, null))
        .catch(() => {});
  };
  const drop = useFileDrop("[data-library-drop]", handlePaths);

  const answer = (choice: Choice) => {
    pending.current = null;
    question?.resolve(choice);
    setQuestion(null);
  };

  const kindLabel = question
    ? t(question.kind === "audio" ? "library.drop.kindAudio" : "library.drop.kindText")
    : "";
  const isAudio = question?.kind === "audio";
  const progress = question?.replaces ? progressOf(question.book, question.kind) : 0;
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
          {/* Ersetzen löscht das vorhandene Hörbuch: zurückhaltend, nicht primär.
              Primär ist das Anhängen, das nichts zerstört. */}
          <Button
            variant={isAudio && question?.replaces ? "secondary" : "primary"}
            onClick={() => answer("attach")}
            data-testid="library-drop-attach"
          >
            {question?.replaces
              ? t(isAudio ? "library.drop.replaceAudio" : "library.drop.replaceText")
              : t("library.drop.attach")}
          </Button>
          {isAudio && question?.replaces && (
            <Button
              variant="primary"
              onClick={() => answer("append")}
              data-testid="library-drop-append"
            >
              {t("library.drop.append")}
            </Button>
          )}
        </>
      }
    >
      {question && (
        <div className="library-add__merge" data-testid="library-drop-question">
          <BookCover book={question.book} size="xs" />
          <div className="library-drop-question__text">
            <strong style={{ overflowWrap: "anywhere" }}>{question.name}</strong>
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
                {isAudio && (
                  <p className="library-meta" data-testid="library-drop-append-hint">
                    {t("library.drop.appendHint", { title: question.book.title })}
                  </p>
                )}
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

  /** Dateien wie auf eine freie Fläche abgelegt aufnehmen (Dialog „Hinzufügen“). */
  const importPaths = (paths: string[]) => handlePaths(paths, null);

  return { drop, working, dialog, importPaths };
}
