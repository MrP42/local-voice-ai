import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, Languages, Play } from "lucide-react";
import { commands, type KiDocument, type LibraryBook } from "@/bindings";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
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
import {
  KI_LANGS,
  defaultKiLang,
  kiLangLabel,
  rememberKiLang,
} from "../settings/tts/kidocs/targetLang";

/** Auswahlwert für „wie das Buch“ (die Liste kennt keinen leeren Wert). */
const SAME_AS_BOOK = "book";

/**
 * Je Sprache das neueste Kurzbuch, die Sprachen in fester Reihenfolge
 * (lva#217). Dokumente ohne Sprachangabe (vor lva#217) stehen unter "".
 */
export function kurzbuchByLanguage(docs: KiDocument[]): KiDocument[] {
  const newest = new Map<string, KiDocument>();
  for (const d of docs) {
    if (d.template_id !== KURZBUCH_TEMPLATE_ID) continue;
    const lang = d.language ?? "";
    const have = newest.get(lang);
    if (!have || d.modified_ms > have.modified_ms) newest.set(lang, d);
  }
  const order = (l: string) => {
    const i = KI_LANGS.findIndex((x) => x.code === l);
    return i < 0 ? KI_LANGS.length : i;
  };
  return [...newest.values()].sort(
    (a, b) => order(a.language ?? "") - order(b.language ?? ""),
  );
}

/**
 * Kurzbuch im Buchdetail (lva#205): öffnen, weiterhören oder erstellen. Ein
 * Hörbuch ohne Text braucht erst sein Transkript. Je Buch sind Kurzbücher in
 * mehreren Sprachen möglich (lva#217): Die Sprache steht am Ergebnis, die
 * Sprache des nächsten Kurzbuchs ist wählbar (vorbelegt mit der zuletzt
 * gewählten, sonst der App-Sprache).
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
  const { t, i18n } = useTranslation();
  const page = book.text?.page_id ?? null;
  const [docs, setDocs] = useState<KiDocument[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [found, setFound] = useState<{ doc: KiDocument; kb: Kurzbuch } | null>(
    null,
  );
  const [lang, setLang] = useState(() => defaultKiLang(i18n.language));
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    setDocs([]);
    setChosen(null);
    setAdding(false);
    if (!page) return;
    let alive = true;
    void commands.bookAiDocuments(page).then((list) => {
      if (!alive || list.status !== "ok" || !Array.isArray(list.data)) return;
      setDocs(kurzbuchByLanguage(list.data));
    });
    return () => {
      alive = false;
    };
  }, [page]);

  const preferred = defaultKiLang(i18n.language);
  // Gezeigt wird die gewählte Sprache, sonst die bevorzugte, sonst die neueste.
  const doc = useMemo(
    () =>
      docs.find((d) => d.id === chosen) ??
      docs.find((d) => (d.language ?? "") === preferred) ??
      [...docs].sort((a, b) => b.modified_ms - a.modified_ms)[0] ??
      null,
    [docs, chosen, preferred],
  );

  useEffect(() => {
    setFound(null);
    if (!page || !doc) return;
    let alive = true;
    void commands.bookAiDocumentText(page, doc.id).then((text) => {
      if (!alive || text.status !== "ok") return;
      const kb = parseKurzbuch(text.data);
      if (kb) setFound({ doc, kb });
    });
    return () => {
      alive = false;
    };
  }, [page, doc]);

  if (!page && !book.audio_path) return null;
  const fraction = found && page ? listenedFraction(page, found.doc.id) : 0;
  // Ältere Kurzbücher (vor lva#217) haben keine Sprachangabe.
  const langOf = (d: KiDocument) =>
    kiLangLabel(d.language) || t("tts.kiDocs.lang.source");

  const create = () => {
    if (!page) return;
    rememberKiLang(lang);
    openKiDocInReader({ page, create: "kurzbuch", lang });
  };
  const langPicker = (
    <div className="library-kurzbuch__lang">
      <Select
        menuPortal
        isClearable={false}
        className="library-kurzbuch__lang-select"
        value={lang || SAME_AS_BOOK}
        options={[
          { value: SAME_AS_BOOK, label: t("tts.kiDocs.lang.source") },
          ...KI_LANGS.map((l) => ({ value: l.code, label: l.label })),
        ]}
        onChange={(v) => v !== null && setLang(v === SAME_AS_BOOK ? "" : v)}
        ariaLabel={t("library.kurzbuch.language")}
      />
      <Button onClick={create} disabled={busy}>
        {found
          ? t("library.kurzbuch.createLang")
          : t("library.kurzbuch.create")}
      </Button>
    </div>
  );

  return (
    <section className="library-kurzbuch" aria-labelledby="kurzbuch-head">
      <div className="library-kurzbuch__info">
        <h3 id="kurzbuch-head">
          {t("library.kurzbuch.title")}
          {found?.doc.language && (
            <span className="library-kurzbuch__badge">{langOf(found.doc)}</span>
          )}
        </h3>
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
        {docs.length > 1 && (
          <div
            role="group"
            aria-label={t("library.kurzbuch.languages")}
            className="library-kurzbuch__langs"
          >
            {docs.map((d) => (
              <button
                key={d.id}
                type="button"
                className="library-kurzbuch__chip"
                aria-pressed={d.id === doc?.id}
                aria-label={t("library.kurzbuch.showLang", { lang: langOf(d) })}
                onClick={() => setChosen(d.id)}
              >
                {langOf(d)}
              </button>
            ))}
          </div>
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
            <Button
              variant="ghost"
              icon={<Languages size={16} aria-hidden="true" />}
              aria-expanded={adding}
              onClick={() => {
                // Vorbelegt: eine Sprache, in der es noch kein Kurzbuch gibt.
                const have = new Set(docs.map((d) => d.language ?? ""));
                if (!adding && have.has(lang))
                  setLang(
                    [preferred, ...KI_LANGS.map((l) => l.code)].find(
                      (c) => !have.has(c),
                    ) ?? lang,
                  );
                setAdding((a) => !a);
              }}
            >
              {t("library.kurzbuch.another")}
            </Button>
          </>
        ) : page ? (
          langPicker
        ) : (
          !transcribing && (
            <Button onClick={onTranscribe} disabled={busy}>
              {t("library.transcribe")}
            </Button>
          )
        )}
      </div>
      {found && page && adding && langPicker}
    </section>
  );
}
