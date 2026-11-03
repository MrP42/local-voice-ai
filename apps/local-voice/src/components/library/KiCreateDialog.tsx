import React, { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Check, Info } from "lucide-react";
import {
  commands,
  type BookTemplate,
  type KiDocument,
  type LibraryBook,
} from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Select } from "../ui/Select";
import { TabList } from "../ui/TabList";
import { TTS_TARGET_LANGS } from "@/lib/constants/languages";
import { KURZBUCH_TEMPLATE_ID } from "@/lib/kurzbuch/kurzbuch";
import { openInReader, openKiDocInReader } from "@/lib/library/books";
import {
  jobsOfBook,
  onBookJobFinished,
  startBookJob,
  useBookJobs,
  type BookJob,
} from "@/lib/library/bookJobs";
import {
  bookLangCode,
  loadPageState,
  type PageState,
} from "@/lib/library/versions";
import {
  KI_LANGS,
  defaultKiLang,
  kiLangLabel,
  rememberKiLang,
} from "../settings/tts/kidocs/targetLang";
import { kiErrorText } from "../settings/tts/kidocs/kiDocs";
import {
  TokenEstimateLine,
  useTokenEstimate,
} from "../settings/tts/kidocs/kiTokens";
import i18n from "@/i18n";
import "../settings/tts/kidocs/kidocs.css";
import "./fassungen.css";

export type KiCreateTab = "translation" | "kurzbuch" | "kidoc";

/** Wie das Buch: die Auswahlliste kennt keinen leeren Wert. */
const SAME_AS_BOOK = "book";
const CATEGORY_ORDER = ["ueberblick", "lernen", "analyse", "anwenden"];

/**
 * Meldung am Ende eines Bibliotheks-Auftrags, wo immer man gerade ist: Erfolg
 * mit „Öffnen“, Fehler mit dem lesbaren Text des Kerns. Einmal registriert.
 */
let toastsBound = false;
export function ensureBookJobToasts() {
  if (toastsBound) return;
  toastsBound = true;
  onBookJobFinished(({ job, ok, error, doc }) => {
    const t = i18n.t.bind(i18n);
    const what = jobTitle(t, job);
    if (!ok) {
      if (
        error === "transform_cancelled" ||
        error === "book_ai:cancelled" ||
        error === "cancelled"
      ) {
        toast(t("library.ki.cancelled", { what }));
        return;
      }
      toast.error(
        t("library.ki.failed", {
          what,
          error: error ? kiErrorText(error, t) : "",
        }),
      );
      return;
    }
    toast.success(t("library.ki.done", { what }), {
      action: {
        label: t("library.ki.open"),
        onClick: () =>
          doc
            ? openKiDocInReader({ page: job.pageId, doc: doc.id, play: false })
            : openInReader(job.pageId, false),
      },
    });
  });
}

type T = (key: string, options?: Record<string, unknown>) => string;

/** „Übersetzung English“, „Kurzbuch Deutsch“, „Lernkarten“. */
export function jobTitle(t: T, job: BookJob): string {
  if (job.kind === "translation")
    return t("library.ki.jobTranslation", {
      lang:
        TTS_TARGET_LANGS.find((l) => l.value === job.target)?.label ??
        job.target,
    });
  const lang = job.lang ? kiLangLabel(job.lang) : "";
  if (job.kind === "kurzbuch")
    return lang
      ? t("library.ki.jobKurzbuchLang", { lang })
      : t("library.ki.jobKurzbuch");
  return lang ? `${job.templateTitle} · ${lang}` : (job.templateTitle ?? "");
}

/** Stand eines Auftrags für Karte und „In Arbeit“. */
export function jobStatus(t: T, job: BookJob): string {
  if (job.status === "queued") return t("library.ki.queued");
  if (job.total > 0)
    return t("library.ki.progress", {
      percent: Math.round((job.done / job.total) * 100),
    });
  return t("library.ki.starting");
}

/**
 * „Mit KI erstellen“ (lva#236): Übersetzung, Kurzbuch und weitere
 * KI-Dokumente direkt aus der Bibliothek starten. Der Auftrag läuft im
 * Hintergrund weiter; die Bibliothek zeigt ihn unter „In Arbeit“ und an der
 * Karte, eine Meldung mit „Öffnen“ kommt, wenn er fertig ist.
 */
export const KiCreateDialog: React.FC<{
  book: LibraryBook | null;
  tab: KiCreateTab;
  onTab(tab: KiCreateTab): void;
  onClose(): void;
  /** Hörbuch ohne Text: zur Buchseite (dort wird transkribiert). */
  onOpenDetail(): void;
}> = ({ book, tab, onTab, onClose, onOpenDetail }) => {
  const { t, i18n: inst } = useTranslation();
  const open = book !== null;
  const pageId = book?.text?.page_id ?? null;
  const [page, setPage] = useState<PageState | null>(null);
  const [docs, setDocs] = useState<KiDocument[]>([]);
  const [templates, setTemplates] = useState<BookTemplate[]>([]);
  const [target, setTarget] = useState<string>("");
  const [lang, setLang] = useState(() => defaultKiLang(inst.language));
  const [templateId, setTemplateId] = useState<string | null>(null);
  const allJobs = useBookJobs();
  const jobs = book ? jobsOfBook(allJobs, book.id) : [];

  useEffect(() => {
    if (!open) return;
    ensureBookJobToasts();
    setPage(null);
    setDocs([]);
    if (!pageId) return;
    let alive = true;
    void loadPageState(pageId).then((p) => alive && setPage(p));
    void commands.bookAiDocuments(pageId).then((r) => {
      if (alive && r.status === "ok" && Array.isArray(r.data)) setDocs(r.data);
    });
    return () => {
      alive = false;
    };
  }, [open, pageId]);

  useEffect(() => {
    if (!open || templates.length) return;
    void commands.bookAiTemplates().then((r) => {
      if (r.status === "ok" && Array.isArray(r.data)) setTemplates(r.data);
    });
  }, [open, templates.length]);

  const bookLang = book && page ? bookLangCode(book, page) : null;
  const targets = useMemo(
    () => TTS_TARGET_LANGS.filter((l) => l.code !== bookLang),
    [bookLang],
  );
  // Vorbelegt: die Sprache der Übersetzung aus den Einstellungen bzw. der App,
  // sonst die erste, die das Buch noch nicht hat.
  useEffect(() => {
    if (!open || !page) return;
    const have = new Set(page.translatedLangs);
    const app = TTS_TARGET_LANGS.find(
      (l) => l.code === inst.language.slice(0, 2),
    )?.value;
    const pick =
      [app, ...targets.map((l) => l.value)].find(
        (v) => v && targets.some((l) => l.value === v) && !have.has(v),
      ) ??
      targets[0]?.value ??
      "";
    setTarget(pick);
  }, [open, page, targets, inst.language]);

  const otherTemplates = useMemo(
    () =>
      templates
        .filter((x) => x.id !== KURZBUCH_TEMPLATE_ID)
        .sort(
          (a, b) =>
            CATEGORY_ORDER.indexOf(a.category) -
            CATEGORY_ORDER.indexOf(b.category),
        ),
    [templates],
  );
  useEffect(() => {
    if (!templateId && otherTemplates.length)
      setTemplateId(otherTemplates[0].id);
  }, [otherTemplates, templateId]);

  const estimateTemplate =
    tab === "kurzbuch"
      ? KURZBUCH_TEMPLATE_ID
      : tab === "kidoc"
        ? templateId
        : null;
  const { estimate, loading } = useTokenEstimate(
    pageId,
    { kind: "book" },
    estimateTemplate,
    open && tab !== "translation" && !!pageId,
  );

  if (!book) return null;

  const translated = page?.translatedLangs ?? [];
  const kurzbuchLangs = docs
    .filter((d) => d.template_id === KURZBUCH_TEMPLATE_ID)
    .map((d) => d.language ?? "");
  const running = (kind: BookJob["kind"], key: string) =>
    jobs.find(
      (j) =>
        j.kind === kind &&
        (kind === "translation" ? j.target === key : (j.lang ?? "") === key) &&
        (kind !== "kidoc" || j.templateId === templateId),
    );

  const langSelect = (
    <label className="ki-create__field">
      <span>{t("library.ki.resultLang")}</span>
      <Select
        menuPortal
        isClearable={false}
        value={lang || SAME_AS_BOOK}
        options={[
          { value: SAME_AS_BOOK, label: t("tts.kiDocs.lang.source") },
          ...KI_LANGS.map((l) => ({ value: l.code, label: l.label })),
        ]}
        onChange={(v) => v !== null && setLang(v === SAME_AS_BOOK ? "" : v)}
        ariaLabel={t("library.ki.resultLang")}
      />
    </label>
  );

  const have = (labels: string[], testId: string) =>
    labels.length > 0 && (
      <div className="ki-create__have" data-testid={testId}>
        <span className="library-meta">{t("library.ki.have")}</span>
        <ul>
          {labels.map((l) => (
            <li key={l}>
              <Check size={12} aria-hidden="true" />
              {l}
            </li>
          ))}
        </ul>
      </div>
    );

  const exists =
    tab === "translation"
      ? translated.includes(target)
      : tab === "kurzbuch"
        ? kurzbuchLangs.includes(lang)
        : false;
  const busy =
    tab === "translation" ? running("translation", target) : running(tab, lang);

  const start = () => {
    if (!pageId) return;
    let result: { started: boolean; queued: boolean };
    let what: string;
    if (tab === "translation") {
      if (!target) return;
      if (exists) {
        // Liegt schon vor: Übersetzen träfe nur den Zwischenspeicher.
        onClose();
        openInReader(pageId, false);
        return;
      }
      const job = {
        bookId: book.id,
        pageId,
        kind: "translation" as const,
        target,
      };
      result = startBookJob(job);
      what = jobTitle(t, {
        ...job,
        id: "",
        status: "queued",
        done: 0,
        total: 0,
      });
    } else {
      const tpl =
        tab === "kurzbuch"
          ? (templates.find((x) => x.id === KURZBUCH_TEMPLATE_ID) ?? null)
          : (otherTemplates.find((x) => x.id === templateId) ?? null);
      if (!tpl && tab === "kidoc") return;
      rememberKiLang(lang);
      const job = {
        bookId: book.id,
        pageId,
        kind: tab,
        templateId: tpl?.id ?? KURZBUCH_TEMPLATE_ID,
        templateTitle: tpl?.title ?? t("library.kurzbuch.title"),
        lang,
      };
      result = startBookJob(job);
      what = jobTitle(t, {
        ...job,
        id: "",
        status: "queued",
        done: 0,
        total: 0,
      });
    }
    toast(
      result.started
        ? t(
            result.queued
              ? "library.ki.queuedToast"
              : "library.ki.startedToast",
            { what },
          )
        : t("library.ki.alreadyRunning", { what }),
    );
    onClose();
  };

  const noText = !pageId;
  const primaryLabel = busy
    ? t("library.ki.running")
    : tab === "translation"
      ? exists
        ? t("library.ki.openReader")
        : t("library.ki.translate")
      : tab === "kurzbuch"
        ? exists
          ? t("library.ki.kurzbuchAgain")
          : t("library.ki.kurzbuch")
        : t("library.ki.create");

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title={t("library.ki.title")}
      description={book.title}
      closeLabel={t("library.cancel")}
      className="ki-create library-dialog-touch"
      footer={
        <>
          {noText ? (
            <Button variant="primary" onClick={onOpenDetail}>
              {t("library.ki.toDetail")}
            </Button>
          ) : (
            <Button
              variant="primary"
              onClick={start}
              disabled={!!busy || (tab === "kidoc" && !templateId)}
              data-testid="ki-create-start"
            >
              {primaryLabel}
            </Button>
          )}
          <Button onClick={onClose}>{t("library.cancel")}</Button>
        </>
      }
    >
      <div className="ki-create__body" data-testid="ki-create">
        <TabList<KiCreateTab>
          tabs={[
            { id: "translation", label: t("library.ki.tabTranslation") },
            { id: "kurzbuch", label: t("library.ki.tabKurzbuch") },
            { id: "kidoc", label: t("library.ki.tabKidoc") },
          ]}
          value={tab}
          onChange={onTab}
          ariaLabel={t("library.ki.title")}
          compact
        />
        {noText ? (
          <p className="ki-create__note" data-testid="ki-create-notext">
            <Info size={16} aria-hidden="true" />
            {book.audio_path
              ? t("library.ki.needsTranscript")
              : t("library.ki.needsText")}
          </p>
        ) : tab === "translation" ? (
          <>
            <p className="library-meta">{t("library.ki.translationHint")}</p>
            <label className="ki-create__field">
              <span>{t("library.ki.targetLang")}</span>
              <Select
                menuPortal
                isClearable={false}
                value={target}
                options={targets.map((l) => ({
                  value: l.value,
                  label: translated.includes(l.value)
                    ? `${l.label} · ${t("library.ki.present")}`
                    : l.label,
                }))}
                onChange={(v) => v !== null && setTarget(v)}
                ariaLabel={t("library.ki.targetLang")}
              />
            </label>
            {have(
              translated.map(
                (v) => TTS_TARGET_LANGS.find((l) => l.value === v)?.label ?? v,
              ),
              "ki-create-have-translations",
            )}
            {exists && (
              <p className="library-meta">
                {t("library.ki.translationExists")}
              </p>
            )}
          </>
        ) : tab === "kurzbuch" ? (
          <>
            <p className="library-meta">{t("library.ki.kurzbuchHint")}</p>
            {langSelect}
            {have(
              kurzbuchLangs.map(
                (l) => kiLangLabel(l) || t("tts.kiDocs.lang.source"),
              ),
              "ki-create-have-kurzbuch",
            )}
            <TokenEstimateLine estimate={estimate} loading={loading} />
          </>
        ) : (
          <>
            <fieldset className="ki-create__templates">
              <legend className="library-meta">
                {t("library.ki.template")}
              </legend>
              {otherTemplates.map((tpl) => (
                <label
                  key={tpl.id}
                  className="ki-create__template"
                  data-testid={`ki-create-template-${tpl.id}`}
                >
                  <input
                    type="radio"
                    name="ki-create-template"
                    checked={templateId === tpl.id}
                    onChange={() => setTemplateId(tpl.id)}
                  />
                  <span>
                    <strong>{tpl.title}</strong>
                    <small>{tpl.description}</small>
                  </span>
                </label>
              ))}
            </fieldset>
            {langSelect}
            <TokenEstimateLine estimate={estimate} loading={loading} />
          </>
        )}
      </div>
    </Dialog>
  );
};
