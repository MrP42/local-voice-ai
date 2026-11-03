import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { Check, ImageIcon, Search } from "lucide-react";
import { commands, type CoverCandidate, type LibraryBook } from "@/bindings";
import { assetUrl } from "@/lib/library/books";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { BookCover } from "./BookCover";

/** Was gewählt ist: das aktuelle Cover, ein Vorschlag oder das generierte. */
type Choice =
  | { kind: "current" }
  | { kind: "generated" }
  | { kind: "candidate"; candidate: CoverCandidate };

const keyOf = (c: CoverCandidate) =>
  c.source === "audio" ? "audio" : `ol-${c.cover_id}`;

/**
 * Cover ändern: zeigt das aktuelle Cover, das in die Hördatei eingebettete
 * Bild und die Cover passender Ausgaben aus Open Library zur Auswahl. Dazu
 * ein eigenes Bild von der Platte und das generierte Cover.
 *
 * Lokal wird sofort gesucht. Open Library fragt der Dialog beim Öffnen
 * einmal an – das Öffnen ist der Knopfdruck, und der Hinweis sagt, was
 * hinausgeht (nur ISBN bzw. Titel und Autor).
 */
export const CoverDialog: React.FC<{
  book: LibraryBook | null;
  onClose(): void;
  /** Nach einer Änderung: Bibliothek neu laden. */
  onChanged(book: LibraryBook): Promise<void> | void;
}> = ({ book, onClose, onChanged }) => {
  const { t } = useTranslation();
  const [candidates, setCandidates] = useState<CoverCandidate[]>([]);
  const [state, setState] = useState<"idle" | "loading" | "done" | "error">(
    "idle",
  );
  const [error, setError] = useState<string | null>(null);
  const [broken, setBroken] = useState<Set<string>>(new Set());
  const [choice, setChoice] = useState<Choice>({ kind: "current" });
  const [busy, setBusy] = useState(false);
  const ticket = useRef(0);

  const search = async (id: string) => {
    const mine = ++ticket.current;
    setState("loading");
    setError(null);
    setBroken(new Set());
    // Erst lokal (schnell, ohne Netz), dann mit Open Library.
    const local = await commands.libraryCoverCandidates(id, false);
    if (mine !== ticket.current) return;
    if (local.status === "ok") setCandidates(local.data);
    const all = await commands.libraryCoverCandidates(id, true);
    if (mine !== ticket.current) return;
    if (all.status === "ok") {
      setCandidates(all.data);
      setState("done");
    } else {
      setError(all.error);
      setState("error");
    }
  };

  useEffect(() => {
    setCandidates([]);
    setBroken(new Set());
    setChoice({ kind: "current" });
    setBusy(false);
    if (!book) {
      ticket.current++;
      return;
    }
    const id = book.id;
    void search(id);
    // Beim Schließen die zwischengelegte Vorschau wegräumen.
    return () => {
      void commands.libraryCoverPreviewClear(id).catch(() => undefined);
    };
    // Nur beim Öffnen für ein anderes Buch neu suchen.
  }, [book?.id]);

  if (!book) return null;

  const visible = candidates.filter((c) => !broken.has(keyOf(c)));
  const hasCover = book.cover_source !== "generated";

  const finish = async (
    task: () => Promise<
      { status: "ok"; data: LibraryBook } | { status: "error"; error: string }
    >,
  ) => {
    setBusy(true);
    try {
      const r = await task();
      if (r.status === "error") {
        setError(r.error);
        return;
      }
      await onChanged(r.data);
      onClose();
    } finally {
      setBusy(false);
    }
  };

  const apply = () => {
    if (choice.kind === "current") return onClose();
    if (choice.kind === "generated")
      return void finish(() => commands.libraryCoverClear(book.id));
    const c = choice.candidate;
    if (c.source === "audio")
      return void finish(() => commands.libraryCoverFromAudio(book.id));
    return void finish(() =>
      commands.libraryCoverFetch(book.id, c.cover_id, c.isbn),
    );
  };

  const pickFile = async () => {
    const picked = await openFile({
      multiple: false,
      filters: [
        {
          name: t("library.coverFiles"),
          extensions: ["jpg", "jpeg", "png", "webp", "epub"],
        },
      ],
    });
    if (typeof picked !== "string") return;
    await finish(() => commands.libraryCoverFromFile(book.id, picked));
  };

  const tile = (
    key: string,
    selected: boolean,
    onSelect: () => void,
    image: React.ReactNode,
    label: string,
    sub?: string,
  ) => (
    <button
      type="button"
      key={key}
      className="library-cover-pick__tile"
      aria-pressed={selected}
      onClick={onSelect}
      data-testid="library-cover-tile"
      data-cover={key}
    >
      <span className="library-cover-pick__img">
        {image}
        {selected && (
          <span className="library-cover-pick__check" aria-hidden="true">
            <Check size={14} />
          </span>
        )}
      </span>
      <strong>{label}</strong>
      {sub && <span className="library-meta">{sub}</span>}
    </button>
  );

  return (
    <Dialog
      open
      onOpenChange={(o) => !o && !busy && onClose()}
      title={t("library.cover.title", { title: book.title })}
      closeLabel={t("library.cancel")}
      className="max-w-4xl"
      footer={
        <>
          <Button
            variant="primary"
            onClick={apply}
            disabled={busy || choice.kind === "current"}
            data-testid="library-cover-apply"
          >
            {t("library.cover.apply")}
          </Button>
          <Button onClick={onClose} disabled={busy}>
            {t("library.cancel")}
          </Button>
        </>
      }
    >
      <div className="library-cover-pick" data-testid="library-cover-dialog">
        <div className="library-bar">
          <Button
            icon={<ImageIcon size={16} aria-hidden="true" />}
            onClick={() => void pickFile()}
            disabled={busy}
            data-testid="library-cover-file"
          >
            {t("library.cover.fromFile")}
          </Button>
          <Button
            variant="ghost"
            icon={<Search size={16} aria-hidden="true" />}
            onClick={() => void search(book.id)}
            disabled={busy || state === "loading"}
          >
            {state === "loading"
              ? t("library.searching")
              : t("library.cover.searchAgain")}
          </Button>
        </div>
        <p className="library-meta">{t("library.cover.privacy")}</p>
        <div
          className="library-cover-pick__grid"
          role="group"
          aria-label={t("library.cover.choices")}
          aria-busy={state === "loading"}
        >
          {tile(
            "current",
            choice.kind === "current",
            () => setChoice({ kind: "current" }),
            <BookCover book={book} size="sm" />,
            t("library.cover.current"),
            hasCover
              ? t(`library.cover.source.${book.cover_source}`, {
                  defaultValue: "",
                })
              : t("library.cover.source.generated"),
          )}
          {visible.map((c) => {
            const key = keyOf(c);
            const src = c.source === "audio" ? assetUrl(c.preview) : c.preview;
            return tile(
              key,
              choice.kind === "candidate" && keyOf(choice.candidate) === key,
              () => setChoice({ kind: "candidate", candidate: c }),
              <img
                src={src ?? undefined}
                alt=""
                draggable={false}
                onError={() => {
                  setBroken((b) => new Set(b).add(key));
                  // Ein gewähltes Bild, das nicht lädt, ist keine Wahl mehr.
                  setChoice((ch) =>
                    ch.kind === "candidate" && keyOf(ch.candidate) === key
                      ? { kind: "current" }
                      : ch,
                  );
                }}
              />,
              c.source === "audio"
                ? t("library.cover.fromAudio")
                : t("library.cover.openLibrary"),
              c.label || undefined,
            );
          })}
          {hasCover &&
            tile(
              "generated",
              choice.kind === "generated",
              () => setChoice({ kind: "generated" }),
              <BookCover book={{ ...book, cover_path: null }} size="sm" />,
              t("library.cover.generated"),
            )}
        </div>
        <p className="library-meta" role="status" aria-live="polite">
          {state === "loading"
            ? t("library.cover.searching")
            : state === "error"
              ? error
              : state === "done" && visible.length === 0
                ? t("library.cover.none")
                : state === "done"
                  ? t("library.cover.found", { count: visible.length })
                  : ""}
        </p>
        {error && state !== "error" && (
          <p className="library-meta" role="alert">
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
