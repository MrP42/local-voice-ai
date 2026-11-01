import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { FileText, Upload } from "lucide-react";
import { commands, type LibraryBook, type MetadataCandidate } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { BookCover } from "./BookCover";
import { MetadataPicker, patchFrom } from "./MetadataPicker";
import {
  extensionOf,
  guessFromFileName,
  guessFromText,
  kindOfFile,
  LIBRARY_FILE_EXTENSIONS,
} from "@/lib/library/books";

interface Picked {
  path: string;
  name: string;
  kind: "text" | "audio";
  /** Extrahierter Text (nur Textfassung). */
  text?: string;
}

const splitAuthors = (s: string) =>
  s
    .split(/\s*[,;]\s*/)
    .map((a) => a.trim())
    .filter(Boolean);

/**
 * Buch oder Hörbuch hinzufügen (lva#152). Erkennt, ob das Werk schon in der
 * Bibliothek steht, und hängt die neue Fassung dann an – ein Buch bleibt ein
 * Eintrag. Metadaten und Cover aus Open Library nur auf Knopfdruck; sonst
 * EPUB-Cover oder generiertes Cover.
 */
export const AddBookDialog: React.FC<{
  open: boolean;
  onOpenChange(open: boolean): void;
  onAdded(book: LibraryBook): void | Promise<void>;
}> = ({ open, onOpenChange, onAdded }) => {
  const { t } = useTranslation();
  const [picked, setPicked] = useState<Picked | null>(null);
  const [title, setTitle] = useState("");
  const [authors, setAuthors] = useState("");
  const [duplicate, setDuplicate] = useState<LibraryBook | null>(null);
  const [attach, setAttach] = useState(true);
  const [candidate, setCandidate] = useState<MetadataCandidate | null>(null);
  const [transcribe, setTranscribe] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) return;
    setPicked(null);
    setTitle("");
    setAuthors("");
    setDuplicate(null);
    setAttach(true);
    setCandidate(null);
    setTranscribe(true);
    setError(null);
  }, [open]);

  // Gibt es das Werk schon? (Titel, Autor, ISBN der gewählten Ausgabe)
  useEffect(() => {
    if (!title.trim()) {
      setDuplicate(null);
      return;
    }
    const handle = window.setTimeout(() => {
      void commands
        .libraryFindDuplicate(
          candidate?.title ?? title,
          candidate?.authors.length ? candidate.authors : splitAuthors(authors),
          candidate?.isbn ?? null,
        )
        .then((r) => setDuplicate(r.status === "ok" ? r.data : null));
    }, 250);
    return () => window.clearTimeout(handle);
  }, [title, authors, candidate]);

  const pick = async () => {
    setError(null);
    const path = await openFile({
      multiple: false,
      filters: [{ name: t("library.files"), extensions: LIBRARY_FILE_EXTENSIONS }],
    });
    if (typeof path !== "string") return;
    const kind = kindOfFile(path);
    if (!kind) {
      setError(t("library.unsupported", { ext: extensionOf(path) }));
      return;
    }
    const name = path.split(/[\\/]/).pop() ?? path;
    let guess = guessFromFileName(path);
    let text: string | undefined;
    if (kind === "text") {
      setBusy("extract");
      const r = await commands.ttsExtractDocument(path);
      setBusy(null);
      if (r.status === "error") {
        setError(r.error);
        return;
      }
      text = r.data;
      guess = guessFromText(text) ?? guess;
    }
    setPicked({ path, name, kind, text });
    setTitle(guess.title);
    setAuthors(guess.authors.join(", "));
  };

  const submit = async () => {
    if (!picked || !title.trim()) return;
    setBusy("save");
    setError(null);
    try {
      const fail = (r: { status: string; error?: string }) => {
        if (r.status === "error") throw new Error(r.error);
      };
      const description =
        candidate?.work_key
          ? await commands
              .libraryDescription(candidate.work_key)
              .then((r) => (r.status === "ok" ? r.data : null))
          : null;
      const base: LibraryBook =
        duplicate && attach
          ? duplicate
          : {
              id: "",
              title: title.trim(),
              subtitle: null,
              authors: splitAuthors(authors),
              language: null,
              isbn: null,
              publisher: null,
              publish_date: null,
              pages: null,
              description: null,
              metadata_source: picked.path.toLowerCase().endsWith(".epub")
                ? "epub"
                : "manual",
              cover_source: "generated",
              text: null,
              audio: null,
              added_ms: 0,
              finished: false,
              cover_path: null,
              audio_path: null,
            };
      let book: LibraryBook = candidate
        ? { ...base, ...patchFrom(candidate, description) }
        : base;

      if (picked.kind === "text" && picked.text !== undefined) {
        const created = await commands.pagesCreate(book.title);
        fail(created);
        if (created.status !== "ok") return;
        await commands.pageStateSave(
          created.data.id,
          JSON.stringify({
            text: picked.text,
            summary: "",
            sourceUrl: "",
            tab: "original",
          }),
        );
        book = {
          ...book,
          text: { page_id: created.data.id, origin: "document", source_name: picked.name },
        };
      }
      const saved = await commands.librarySave(book);
      fail(saved);
      if (saved.status !== "ok") return;
      book = saved.data;

      if (picked.kind === "audio") {
        setBusy("copy");
        const imported = await commands.libraryAudioImport(book.id, picked.path);
        fail(imported);
        if (imported.status !== "ok") return;
        book = imported.data;
        if (transcribe && !book.text && book.audio_path && book.audio) {
          // Ein Hörbuch, keine Gesprächsaufnahme: keine Einwilligung nötig.
          const job = await commands.meetingsImportFile(book.audio_path, true, null);
          if (job.status === "ok") {
            const withJob = await commands.librarySave({
              ...book,
              audio: { ...book.audio, transcript_job: job.data },
            });
            if (withJob.status === "ok") book = withJob.data;
          } else toast.error(job.error);
        }
      }

      if (candidate?.cover_id || (candidate?.isbn && book.cover_source === "generated")) {
        const r = await commands.libraryCoverFetch(
          book.id,
          candidate.cover_id ?? null,
          candidate.isbn ?? null,
        );
        if (r.status === "ok") book = r.data;
      } else if (
        book.cover_source === "generated" &&
        picked.path.toLowerCase().endsWith(".epub")
      ) {
        const r = await commands.libraryCoverFromFile(book.id, picked.path);
        if (r.status === "ok") book = r.data;
      }
      toast(
        duplicate && attach
          ? t("library.attached", { title: book.title })
          : t("library.added", { title: book.title }),
      );
      onOpenChange(false);
      await onAdded(book);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const hasSameKind =
    duplicate &&
    (picked?.kind === "text" ? !!duplicate.text : !!duplicate.audio);

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("library.addTitle")}
      closeLabel={t("library.cancel")}
      contentClassName="library-detail"
      footer={
        <>
          <Button onClick={() => onOpenChange(false)}>{t("library.cancel")}</Button>
          <Button
            variant="primary"
            onClick={() => void submit()}
            disabled={!picked || !title.trim() || busy !== null}
            data-testid="library-add-submit"
          >
            {busy === "copy"
              ? t("library.copying")
              : duplicate && attach
                ? t("library.attach")
                : t("library.add")}
          </Button>
        </>
      }
    >
      <div className="library-add__drop">
        {picked ? (
          <span style={{ display: "flex", gap: 8, alignItems: "center", minWidth: 0 }}>
            <FileText size={18} aria-hidden="true" />
            <span style={{ minWidth: 0 }}>
              <strong style={{ overflowWrap: "anywhere" }}>{picked.name}</strong>
              <br />
              <span className="library-meta">
                {picked.kind === "audio" ? t("library.kindAudio") : t("library.kindText")}
              </span>
            </span>
          </span>
        ) : (
          <span className="library-meta">{t("library.pickHint")}</span>
        )}
        <span className="library-bar__spacer" />
        <Button
          icon={<Upload size={16} aria-hidden="true" />}
          onClick={() => void pick()}
          disabled={busy !== null}
          data-testid="library-add-pick"
        >
          {busy === "extract"
            ? t("library.reading")
            : picked
              ? t("library.pickOther")
              : t("library.pick")}
        </Button>
      </div>

      {error && (
        <p className="library-meta" role="alert" style={{ color: "var(--color-danger, #c50f1f)" }}>
          {error}
        </p>
      )}

      {picked && (
        <>
          <div className="library-form">
            <label>
              {t("library.field.title")}
              <Input
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                data-testid="library-add-title"
              />
            </label>
            <label>
              {t("library.field.authors")}
              <Input
                value={authors}
                onChange={(e) => setAuthors(e.target.value)}
                data-testid="library-add-authors"
              />
            </label>
          </div>

          {duplicate && (
            <div className="library-add__merge" data-testid="library-duplicate">
              <BookCover book={duplicate} size="xs" />
              <div>
                <strong>{t("library.dupTitle")}</strong>
                <div className="library-meta">
                  {hasSameKind
                    ? t(
                        picked.kind === "text"
                          ? "library.dupReplaceText"
                          : "library.dupReplaceAudio",
                        { title: duplicate.title },
                      )
                    : t(
                        picked.kind === "text"
                          ? "library.dupAttachText"
                          : "library.dupAttachAudio",
                        { title: duplicate.title },
                      )}
                </div>
                <div className="library-bar" style={{ marginTop: 6 }}>
                  <label className="library-meta" style={{ display: "flex", gap: 6 }}>
                    <input
                      type="radio"
                      name="library-merge"
                      checked={attach}
                      onChange={() => setAttach(true)}
                    />
                    {t("library.dupAttach")}
                  </label>
                  <label className="library-meta" style={{ display: "flex", gap: 6 }}>
                    <input
                      type="radio"
                      name="library-merge"
                      checked={!attach}
                      onChange={() => setAttach(false)}
                    />
                    {t("library.dupSeparate")}
                  </label>
                </div>
              </div>
            </div>
          )}

          <section aria-labelledby="library-add-meta">
            <h3 id="library-add-meta" className="library-section__head">
              {t("library.coverAndMeta")}
              <small>{t("library.privacyHint")}</small>
            </h3>
            <MetadataPicker
              title={title}
              authors={splitAuthors(authors)}
              onApply={() => {}}
              onSelect={setCandidate}
            />
            <p className="library-meta" style={{ marginTop: 6 }}>
              {candidate
                ? t("library.candidateChosen", { title: candidate.title })
                : picked.path.toLowerCase().endsWith(".epub")
                  ? t("library.coverEpub")
                  : t("library.coverGeneratedHint")}
            </p>
          </section>

          {picked.kind === "audio" && !(duplicate && attach && duplicate.text) && (
            <label className="library-meta" style={{ display: "flex", gap: 8 }}>
              <input
                type="checkbox"
                checked={transcribe}
                onChange={(e) => setTranscribe(e.target.checked)}
                data-testid="library-add-transcribe"
              />
              {t("library.transcribeOption")}
            </label>
          )}
        </>
      )}
    </Dialog>
  );
};
