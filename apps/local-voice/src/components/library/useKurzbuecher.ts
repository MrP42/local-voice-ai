import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type KiDocument, type LibraryBook } from "@/bindings";
import { kurzbuchByLanguage } from "./KurzbuchCard";
import { defaultKiLang } from "../settings/tts/kidocs/targetLang";

/** Vorhandenes Kurzbuch eines Buchs für die Bibliothekskarte. */
export interface KurzbuchInfo {
  page: string;
  /** Gezeigtes Kurzbuch: bevorzugte Sprache, sonst das neueste. */
  doc: KiDocument;
  /** Anzahl der Sprachen, in denen es ein Kurzbuch gibt. */
  langs: number;
}

/**
 * Kurzbücher aller Bücher mit Textfassung (Patrick 04.11.: auf der Karte
 * sichtbar und direkt startbar). Lädt neu, wenn sich die Bücher ändern oder
 * ein Kurzbuch fertig wird (`lv-kidocs-changed`).
 */
export function useKurzbuecher(
  books: LibraryBook[] | null | undefined,
): Record<string, KurzbuchInfo> {
  const { i18n } = useTranslation();
  const preferred = defaultKiLang(i18n.language);
  const pages = useMemo(
    () =>
      (books ?? [])
        .filter((b) => b.text?.page_id)
        .map((b) => [b.id, b.text!.page_id] as const),
    [books],
  );
  const pagesKey = pages.map(([id, p]) => `${id}:${p}`).join(",");
  const [docs, setDocs] = useState<Record<string, KiDocument[]>>({});
  const [rev, setRev] = useState(0);

  useEffect(() => {
    const on = () => setRev((n) => n + 1);
    window.addEventListener("lv-kidocs-changed", on);
    return () => window.removeEventListener("lv-kidocs-changed", on);
  }, []);

  useEffect(() => {
    let alive = true;
    void Promise.all(
      pages.map(async ([id, page]) => {
        try {
          const list = await commands.bookAiDocuments(page);
          return [
            id,
            list.status === "ok" && Array.isArray(list.data)
              ? kurzbuchByLanguage(list.data)
              : [],
          ] as const;
        } catch {
          return [id, []] as const;
        }
      }),
    ).then((entries) => {
      if (alive) setDocs(Object.fromEntries(entries));
    });
    return () => {
      alive = false;
    };
    // pagesKey fasst `pages` zusammen; rev: neu laden nach einem Auftrag.
  }, [pagesKey, rev]);

  return useMemo(() => {
    const out: Record<string, KurzbuchInfo> = {};
    for (const [id, page] of pages) {
      const list = docs[id];
      if (!list?.length) continue;
      const doc =
        list.find((d) => (d.language ?? "") === preferred) ??
        [...list].sort((a, b) => b.modified_ms - a.modified_ms)[0];
      out[id] = { page, doc, langs: list.length };
    }
    return out;
  }, [pages, docs, preferred]);
}
