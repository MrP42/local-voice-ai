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
import { MetadataPicker } from "./MetadataPicker";
import {
  extensionOf,
  kindOfFile,
  LIBRARY_FILE_EXTENSIONS,
} from "@/lib/library/books";
import {
  addToLibrary,
  prepareFile,
  type PreparedFile,
} from "@/lib/library/importFile";


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
  const [picked, setPicked] = useState<PreparedFile | null>(null);
  const [title, setTitle] = useState("");
  const [authors, setAuthors] = useState("");
  const [duplicate, setDuplicate] = useState<LibraryBook | null>(null);
  /** Dublettenprüfung läuft: so lange kein Absenden (sonst falsches Werk). */
  const [checking, setChecking] = useState(false);
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
      setChecking(false);
      return;
    }
    let stale = false;
    setChecking(true);
    const handle = window.setTimeout(() => {
      void commands
        .libraryFindDuplicate(
          candidate?.title ?? title,
          candidate?.authors.length ? candidate.authors : splitAuthors(authors),
          candidate?.isbn ?? null,
        )
        .then((r) => {
          if (stale) return;
          setDuplicate(r.status === "ok" ? r.data : null);
          setChecking(false);
        });
    }, 250);
    return () => {
      stale = true;
      window.clearTimeout(handle);
    };
  }, [title, authors, candidate]);

  const pick = async () => {
    setError(null);
    const path = await openFile({
      multiple: false,
      filters: [{ name: t("library.files"), extensions: LIBRARY_FILE_EXTENSIONS }],
    });
    if (typeof path !== "string") return;
    if (!kindOfFile(path)) {
      setError(t("library.unsupported", { ext: extensionOf(path) }));
      return;
    }
    setBusy("extract");
    try {
      const file = await prepareFile(path);
      setPicked(file);
      setTitle(file.title);
      setAuthors(file.authors.join(", "));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const submit = async () => {
    if (!picked || !title.trim()) return;
    setBusy("save");
    setError(null);
    try {
      const { book, attached } = await addToLibrary(picked, {
        attachTo: duplicate && attach ? duplicate : null,
        title: title.trim(),
        authors: splitAuthors(authors),
        candidate,
        transcribe,
        onWarning: (m) => toast.error(m),
      });
      toast(
        attached
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
            disabled={!picked || !title.trim() || busy !== null || checking}
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
