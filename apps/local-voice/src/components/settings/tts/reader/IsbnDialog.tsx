import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../../../ui/Dialog";
import { Button } from "../../../ui/Button";
import { Input } from "../../../ui/Input";
import { openUrl } from "@tauri-apps/plugin-opener";
import { commands, type IsbnBook } from "@/bindings";

interface IsbnDialogProps {
  open: boolean;
  onOpenChange(open: boolean): void;
  /** Vorschlag aus dem Text (z. B. „ISBN 978-…“ im Impressum). */
  initialIsbn: string;
  /** Erst nach Bestätigung: Titel und Autoren übernehmen. */
  onApply(book: IsbnBook): void;
}

type Lookup =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "found"; book: IsbnBook }
  | { state: "none" }
  | { state: "error"; message: string };

/**
 * Buchdaten per ISBN aus Open Library (lva#149). Gesucht wird nur auf
 * Knopfdruck, und es geht allein die ISBN ins Netz. Der Treffer wird angezeigt
 * und erst mit „Übernehmen“ in die Titelei geschrieben — es kann eine andere
 * Ausgabe sein.
 */
export const IsbnDialog: React.FC<IsbnDialogProps> = ({
  open,
  onOpenChange,
  initialIsbn,
  onApply,
}) => {
  const { t } = useTranslation();
  const [isbn, setIsbn] = useState(initialIsbn);
  const [lookup, setLookup] = useState<Lookup>({ state: "idle" });

  useEffect(() => {
    if (!open) return;
    setIsbn(initialIsbn);
    setLookup({ state: "idle" });
  }, [open, initialIsbn]);

  const search = async () => {
    if (!isbn.trim() || lookup.state === "loading") return;
    setLookup({ state: "loading" });
    const result = await commands.isbnLookup(isbn);
    if (result.status === "error")
      setLookup({ state: "error", message: String(result.error) });
    else if (!result.data) setLookup({ state: "none" });
    else setLookup({ state: "found", book: result.data });
  };

  const found = lookup.state === "found" ? lookup.book : null;
  const facts = found
    ? [
        found.publishers.join(", "),
        found.publish_date,
        found.pages ? t("tts.isbn.pages", { count: found.pages }) : null,
      ].filter(Boolean)
    : [];

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.isbn.title")}
      closeLabel={t("tts.isbn.cancel")}
      footer={
        <>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("tts.isbn.cancel")}
          </Button>
          {found ? (
            <Button
              variant="primary"
              data-testid="isbn-apply"
              onClick={() => {
                onApply(found);
                onOpenChange(false);
              }}
            >
              {t("tts.isbn.apply")}
            </Button>
          ) : (
            <Button
              variant="primary"
              data-testid="isbn-search"
              onClick={() => void search()}
              disabled={!isbn.trim() || lookup.state === "loading"}
            >
              {lookup.state === "loading"
                ? t("tts.isbn.searching")
                : t("tts.isbn.search")}
            </Button>
          )}
        </>
      }
    >
      <div className="space-y-3">
        <p className="text-sm text-text/70">{t("tts.isbn.privacy")}</p>
        <label className="block text-sm">
          <span className="mb-1 block font-medium">{t("tts.isbn.label")}</span>
          <Input
            type="text"
            value={isbn}
            onChange={(e) => {
              setIsbn(e.target.value);
              if (lookup.state !== "loading") setLookup({ state: "idle" });
            }}
            placeholder="978-3-…"
            className="w-full"
            autoFocus
            inputMode="text"
            data-testid="isbn-input"
            onKeyDown={(e) => {
              if (e.key === "Enter") void search();
            }}
          />
        </label>
        <div aria-live="polite" data-testid="isbn-result">
          {lookup.state === "none" && (
            <p className="text-sm">{t("tts.isbn.none")}</p>
          )}
          {lookup.state === "error" && (
            <p className="text-sm text-red-600 dark:text-red-400">
              {lookup.message}
            </p>
          )}
          {found && (
            <div className="rounded-md border border-mid-gray/30 p-3">
              <p className="font-semibold">{found.title}</p>
              {found.subtitle && (
                <p className="text-sm text-text/80">{found.subtitle}</p>
              )}
              {found.authors.length > 0 && (
                <p className="mt-1 text-sm">
                  {t("tts.isbn.by", { authors: found.authors.join(", ") })}
                </p>
              )}
              {facts.length > 0 && (
                <p className="mt-1 text-xs text-text/70">{facts.join(" · ")}</p>
              )}
              {found.url && (
                <button
                  type="button"
                  onClick={() => found.url && void openUrl(found.url)}
                  className="mt-1 inline-block cursor-pointer text-xs text-logo-primary underline"
                >
                  {t("tts.isbn.openLibrary")}
                </button>
              )}
              <p className="mt-2 text-xs text-text/70">{t("tts.isbn.check")}</p>
            </div>
          )}
        </div>
      </div>
    </Dialog>
  );
};
