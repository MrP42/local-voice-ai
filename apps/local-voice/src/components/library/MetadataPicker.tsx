import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Search } from "lucide-react";
import { commands, type LibraryBook, type MetadataCandidate } from "@/bindings";
import { Button } from "../ui/Button";

const coverUrl = (id: number) => `https://covers.openlibrary.org/b/id/${id}-M.jpg`;

/** Sprache aus Open Library (ISO 639-2) in Klartext, sonst unverändert. */
const LANG: Record<string, string> = {
  ger: "Deutsch",
  eng: "Englisch",
  fre: "Französisch",
  spa: "Spanisch",
  ita: "Italienisch",
};

export type MetadataPatch = Partial<
  Pick<
    LibraryBook,
    | "title"
    | "subtitle"
    | "authors"
    | "isbn"
    | "publisher"
    | "publish_date"
    | "pages"
    | "language"
    | "description"
    | "metadata_source"
  >
>;

/** Kandidat als Änderung am Buch; leere Felder bleiben unberührt. */
export function patchFrom(c: MetadataCandidate, description: string | null): MetadataPatch {
  const patch: MetadataPatch = { title: c.title, metadata_source: "openlibrary" };
  if (c.subtitle) patch.subtitle = c.subtitle;
  if (c.authors.length) patch.authors = c.authors;
  if (c.isbn) patch.isbn = c.isbn;
  if (c.publisher) patch.publisher = c.publisher;
  if (c.first_publish_year) patch.publish_date = String(c.first_publish_year);
  if (c.pages) patch.pages = c.pages;
  if (c.language) patch.language = LANG[c.language] ?? c.language;
  if (description) patch.description = description;
  return patch;
}

/**
 * Ausgaben bei Open Library suchen und eine übernehmen (lva#152). Gesucht
 * wird nur auf Knopfdruck; hinaus gehen allein ISBN oder Titel und Autor.
 * Übernommen wird erst nach Auswahl und Bestätigung.
 */
export const MetadataPicker: React.FC<{
  title: string;
  authors: string[];
  isbn?: string | null;
  /** Ausgewählt und bestätigt: Metadaten und Cover-Kennung. */
  onApply(patch: MetadataPatch, coverId: number | null): void;
  /** Statt eines Bestätigungsknopfs: Auswahl sofort melden (Dialog). */
  onSelect?(candidate: MetadataCandidate | null): void;
}> = ({ title, authors, isbn, onApply, onSelect }) => {
  const { t } = useTranslation();
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "loading" }
    | { kind: "done"; hits: MetadataCandidate[] }
    | { kind: "error"; message: string }
  >({ kind: "idle" });
  const [chosen, setChosen] = useState<number | null>(null);

  const search = async () => {
    setState({ kind: "loading" });
    setChosen(null);
    onSelect?.(null);
    const r = await commands.libraryMetadataSearch(
      title,
      authors.join(" "),
      isbn ?? null,
    );
    if (r.status === "error") setState({ kind: "error", message: r.error });
    else setState({ kind: "done", hits: r.data });
  };

  const choose = (i: number | null) => {
    setChosen(i);
    if (state.kind === "done") onSelect?.(i === null ? null : state.hits[i]);
  };

  const apply = async () => {
    if (state.kind !== "done" || chosen === null) return;
    const c = state.hits[chosen];
    const description = c.work_key
      ? await commands.libraryDescription(c.work_key).then((r) =>
          r.status === "ok" ? r.data : null,
        )
      : null;
    onApply(patchFrom(c, description), c.cover_id);
  };

  return (
    <div className="library-detail" style={{ gap: 10 }}>
      <div className="library-bar">
        <Button
          icon={<Search size={16} aria-hidden="true" />}
          onClick={() => void search()}
          disabled={state.kind === "loading" || !title.trim()}
          data-testid="library-meta-search"
        >
          {state.kind === "loading" ? t("library.searching") : t("library.searchOpenLibrary")}
        </Button>
        {!onSelect && state.kind === "done" && chosen !== null && (
          <Button variant="primary" onClick={() => void apply()}>
            {t("library.apply")}
          </Button>
        )}
      </div>
      {state.kind === "error" && (
        <p className="library-meta" role="alert">
          {state.message}
        </p>
      )}
      {state.kind === "done" && state.hits.length === 0 && (
        <p className="library-meta">{t("library.noHits")}</p>
      )}
      {state.kind === "done" && state.hits.length > 0 && (
        <div className="library-add__cands" role="group" aria-label={t("library.editions")}>
          {state.hits.map((c, i) => (
            <button
              type="button"
              key={`${c.work_key}-${i}`}
              className="library-add__cand"
              aria-pressed={chosen === i}
              onClick={() => choose(chosen === i ? null : i)}
              data-testid="library-candidate"
            >
              {c.cover_id ? (
                <img src={coverUrl(c.cover_id)} alt="" loading="lazy" />
              ) : (
                <span
                  className="book-cover"
                  style={{ background: "var(--lib-line)" }}
                  aria-hidden="true"
                />
              )}
              <strong>{c.title}</strong>
              <span className="library-meta">
                {[c.authors[0], c.first_publish_year, c.publisher]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
};
