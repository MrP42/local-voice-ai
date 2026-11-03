import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { DocUsageLine } from "./DocUsageLine";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import {
  commands,
  events,
  type DocTypeInfo,
  type MeetingDocument,
  type MinutesMeta,
  type MinutesProgress,
} from "@/bindings";
import {
  minutesErrorCode,
  minutesErrorDetail,
  phaseKey,
  progressPercent,
} from "@/lib/meetingMinutes";
import { Alert } from "../../ui/Alert";
import Badge from "../../ui/Badge";
import { IconAction } from "../../ui/IconAction";
import {
  MarkdownContent,
  type SlideRefs,
} from "../../whats-new/MarkdownContent";
import { defaultDocBasis } from "./language/DocBasisFields";
import { ReadAloudButton } from "./ReadAloudButton";
import {
  Check,
  ChevronDown,
  Copy,
  Download,
  FileText,
  History,
  Pencil,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import { ActionMenu } from "../../ui/ActionMenu";
import { locateInMarkdown } from "@/lib/minutesEdit";
import { MinutesEditor, minutesDraftFor } from "./MinutesEditor";
import { useRegenerateGuard } from "./useRegenerateGuard";

/** Blöcke der Leseansicht, die einzeln zum Bearbeiten angeklickt werden können. */
const EDIT_BLOCKS = "h1, h2, h3, p, li, td, th, blockquote";
/** Abschnitte mit eigenem, immer sichtbarem „Abschnitt bearbeiten“. */
const SECTION_HEADINGS = "h2, h3";

/** Nächster Vorfahr, der senkrecht scrollt (Reiterfläche der Besprechung). */
const scrollParentOf = (el: HTMLElement | null): HTMLElement | null => {
  for (let node = el?.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if (overflow === "auto" || overflow === "scroll") return node;
  }
  return null;
};

interface MinutesViewProps {
  meetingId: string;
  meetingTitle: string;
  /** D5: Folienbelege (`[F7]`) im Protokoll anklickbar machen (Sprung zur Folie). */
  slideRefs?: SlideRefs;
  /**
   * Dokumenttyp (Management Summary, Kurzfassung, ...). Ohne: das Protokoll. Die
   * Ansicht bleibt dieselbe; es wechseln nur `kind`, Erzeugen, Ereignis und Dateiname.
   */
  docType?: DocTypeInfo;
  /** Vorne in der Werkzeugzeile (Typleiste der Dokumente): keine eigene Zeile. */
  toolbarLead?: React.ReactNode;
}

export const MinutesView: React.FC<MinutesViewProps> = ({
  meetingId,
  meetingTitle,
  slideRefs,
  docType,
  toolbarLead,
}) => {
  const { t, i18n } = useTranslation();
  const docKind = docType ? docType.kind : "minutes";
  const docKey = docType?.key ?? null;
  // Alle Versionen dieses Typs, neueste zuerst; angezeigt wird die gewählte
  // (Standard: die neueste). Neu erzeugen legt eine Version an, eine
  // Handbearbeitung ändert die angezeigte Version.
  const [versions, setVersions] = useState<MeetingDocument[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const doc = versions.find((d) => d.id === selectedId) ?? versions[0] ?? null;
  const setDoc = useCallback((next: MeetingDocument | null) => {
    if (!next) return;
    setVersions((list) =>
      [next, ...list.filter((d) => d.id !== next.id)].sort(
        (a, b) => b.version - a.version,
      ),
    );
  }, []);
  // Bearbeiten: `null` = Leseansicht; sonst die Stelle, an der der Editor öffnet.
  const [editing, setEditing] = useState<{
    reveal: { start: number; end: number } | null;
  } | null>(null);
  // Stift neben dem Block unter dem Mauszeiger (wie beim Transkript).
  const [hoverPen, setHoverPen] = useState<{
    top: number;
    block: HTMLElement;
  } | null>(null);
  // Abschnittsknöpfe neben jeder Überschrift (Lage in der Vorschau gemessen).
  const [sections, setSections] = useState<
    { top: number; el: HTMLElement; title: string }[]
  >([]);
  // Unten geht es weiter: Hinweis am unteren Rand der Reiterfläche.
  const [moreBelow, setMoreBelow] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const guard = useRegenerateGuard();
  // Sperrt "Neu erzeugen" schon während der Rückfrage (Doppelklick).
  const generatingRef = useRef(false);
  const [loading, setLoading] = useState(true);
  // Der Lauf gehoert dem Backend (B14): `running` kommt aus dessen Zustand und
  // seinen Ereignissen, nicht aus dem Aufruf dieses Reiters. So bleibt der
  // Knopf gesperrt, wenn man den Reiter verlaesst und wieder oeffnet.
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<MinutesProgress | null>(null);
  const [meta, setMeta] = useState<MinutesMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // The rendered preview doubles as the source for the formatted clipboard
  // copy: reading its innerHTML guarantees that what lands in Word is exactly
  // what the user sees here, instead of a second Markdown renderer that could
  // drift from the first.
  const previewRef = useRef<HTMLDivElement>(null);

  // Neu laden nach einem Lauf darf den offenen Editor weder aushängen noch
  // die bearbeitete Version wechseln: der Ladetext steht nur beim ersten Laden.
  const editingRef = useRef(false);
  editingRef.current = editing !== null;
  const loadedOnce = useRef(false);
  const loadLatest = useCallback(async () => {
    if (!loadedOnce.current) setLoading(true);
    const result = await commands.meetingsGetDocuments(meetingId);
    loadedOnce.current = true;
    setLoading(false);
    if (result.status !== "ok") {
      // Failing to load used to render as "no minutes yet" — which is exactly
      // what a deleted document looks like. Say which of the two it is.
      setError(t("meetings.minutes.loadError", { error: result.error }));
      return;
    }
    setError(null);
    const minutes = result.data
      .filter((d) => d.kind === docKind)
      .sort((a, b) => b.version - a.version);
    setVersions(minutes);
    if (editingRef.current) return;
    // Offener Entwurf (Reiter gewechselt, Besprechung verlassen): wieder öffnen.
    const drafted = minutes.find((d) => minutesDraftFor(d.id) !== undefined);
    if (drafted) {
      setSelectedId(drafted.id);
      setEditing({ reveal: null });
      return;
    }
    setSelectedId(null);
  }, [meetingId, t, docKind]);

  const loadMeta = useCallback(async () => {
    const result = docKey
      ? await commands.meetingsDocumentMeta(meetingId, docKey)
      : await commands.meetingsMinutesMeta(meetingId);
    setMeta(result.status === "ok" ? (result.data ?? null) : null);
  }, [meetingId, docKey]);

  // Beim Einblenden fragen, ob schon ein Lauf besteht (Reiterwechsel).
  const loadRunState = useCallback(async () => {
    const result = await commands.meetingsMinutesState(meetingId);
    if (result.status !== "ok" || !result.data) return;
    setRunning(result.data.running);
    setProgress(result.data.running ? (result.data.progress ?? null) : null);
  }, [meetingId]);

  useEffect(() => {
    void loadLatest();
    void loadMeta();
    void loadRunState();
  }, [loadLatest, loadMeta, loadRunState]);

  const errorText = useCallback(
    (raw: string) =>
      t(`meetings.minutes.errors.${minutesErrorCode(raw)}`, {
        error: minutesErrorDetail(raw),
        defaultValue: raw,
      }),
    [t],
  );

  // Fortschritt, Ende und Fehler des Laufs -- auch eines, den ein anderer
  // Aufruf oder ein frueher geoeffneter Reiter gestartet hat.
  useEffect(() => {
    // Ein Dokumenttyp hoert auf seine eigenen Ereignisse; ein Protokoll-Lauf
    // belegt dieselbe Besprechung (eine Sperre), deshalb fragt er nach dessen Ende
    // den Zustand neu ab.
    if (docKey) {
      const unDoc = events.documentEvent.listen((event) => {
        const payload = event.payload;
        if (payload.meeting_id !== meetingId || payload.doc_type !== docKey)
          return;
        if (payload.kind === "progress") {
          setRunning(true);
          setProgress({
            phase: payload.phase,
            done: payload.done,
            total: payload.total,
          });
        } else if (payload.kind === "done") {
          setRunning(false);
          setProgress(null);
          setError(null);
          void loadLatest();
          void loadMeta();
        } else {
          setRunning(false);
          setProgress(null);
          if (payload.code !== "minutes_cancelled") {
            setError(errorText(payload.code));
          }
        }
      });
      const unMinutes = events.minutesEvent.listen((event) => {
        if (event.payload.meeting_id !== meetingId) return;
        if (event.payload.kind !== "progress") void loadRunState();
      });
      return () => {
        void unDoc.then((f) => f());
        void unMinutes.then((f) => f());
      };
    }
    const un = events.minutesEvent.listen((event) => {
      const payload = event.payload;
      if (payload.meeting_id !== meetingId) return;
      if (payload.kind === "progress") {
        setRunning(true);
        setProgress({
          phase: payload.phase,
          done: payload.done,
          total: payload.total,
        });
      } else if (payload.kind === "done") {
        setRunning(false);
        setProgress(null);
        setError(null);
        void loadLatest();
        void loadMeta();
      } else {
        setRunning(false);
        setProgress(null);
        // Ein Stopp ist kein Fehler.
        if (payload.code !== "minutes_cancelled") {
          setError(errorText(payload.code));
        }
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [meetingId, docKey, loadLatest, loadMeta, loadRunState, errorText]);

  const generate = async () => {
    if (editing || generatingRef.current) return;
    generatingRef.current = true;
    try {
      await runGenerate();
    } finally {
      generatingRef.current = false;
    }
  };

  const runGenerate = async () => {
    // Von Hand bearbeitet? Vorher fragen; die bearbeitete Version bleibt erhalten.
    if (!(await guard.confirm(meetingId, docKind))) return;
    setRunning(true);
    setProgress(null);
    setError(null);
    setSaved(null);
    // `null`: die Vorlage, die fuer diese Besprechung gewaehlt ist (Menue, Vorlage).
    // G5: aktive Fassung, Sprache = letzte Wahl bzw. die der App.
    const result = docKey
      ? await commands.meetingsGenerateDocument(
          meetingId,
          docKey,
          defaultDocBasis(i18n.language),
        )
      : await commands.meetingsGenerateMinutes(
          meetingId,
          null,
          defaultDocBasis(i18n.language),
        );
    if (result.status === "error") {
      // Ein zweiter Start wird abgewiesen, der erste Lauf laeuft weiter: der
      // Reiter zeigt ihn weiter als laufend, kein Fehler.
      if (minutesErrorCode(result.error) === "minutes_busy") {
        void loadRunState();
        return;
      }
      setRunning(false);
      setProgress(null);
      if (minutesErrorCode(result.error) !== "minutes_cancelled") {
        setError(errorText(result.error));
      }
      return;
    }
    setRunning(false);
    setProgress(null);
    setDoc(result.data);
    setSelectedId(null);
    void loadMeta();
  };

  /** Bearbeiten öffnen; mit Block: der Editor zeigt und markiert dessen Stelle. */
  const startEdit = (block?: HTMLElement | null) => {
    if (!doc || running) return;
    const text = block?.textContent ?? "";
    setHoverPen(null);
    // Die bearbeitete Version bleibt angezeigt, auch wenn inzwischen eine neue entsteht.
    setSelectedId(doc.id);
    setEditing({ reveal: text ? locateInMarkdown(doc.body, text) : null });
  };

  /** Editor zu: die neueste Version wieder automatisch zeigen, eine ältere bleibt gewählt. */
  const closeEditor = () => {
    setEditing(null);
    setSelectedId((id) => (id === versions[0]?.id ? null : id));
  };

  const blockAt = (target: EventTarget | null): HTMLElement | null => {
    const el = target instanceof Element ? target : null;
    if (!el || el.closest("[data-slide-ref], a, button, input")) return null;
    const block = el.closest<HTMLElement>(EDIT_BLOCKS);
    return block && previewRef.current?.contains(block) ? block : null;
  };

  // Überschriften messen: bei jeder Größenänderung der Vorschau neu.
  const previewKey = editing ? null : (doc?.id ?? null);
  const docBody = doc?.body;
  useLayoutEffect(() => {
    const preview = previewRef.current;
    const wrapper = wrapperRef.current;
    if (!preview || !wrapper) {
      setSections([]);
      return;
    }
    const measure = () => {
      const base = wrapper.getBoundingClientRect().top;
      setSections(
        Array.from(preview.querySelectorAll<HTMLElement>(SECTION_HEADINGS)).map(
          (el) => {
            const rect = el.getBoundingClientRect();
            return {
              el,
              top: rect.top - base + (rect.height - 32) / 2,
              title: el.textContent?.trim() ?? "",
            };
          },
        ),
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(preview);
    return () => observer.disconnect();
  }, [previewKey, docBody]);

  // Weiter unten Inhalt? Am scrollenden Vorfahren beobachten.
  useEffect(() => {
    const scroller = scrollParentOf(wrapperRef.current);
    if (!scroller || !previewKey) {
      setMoreBelow(false);
      return;
    }
    const check = () =>
      setMoreBelow(
        scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight > 8,
      );
    check();
    scroller.addEventListener("scroll", check, { passive: true });
    const observer = new ResizeObserver(check);
    observer.observe(scroller);
    if (previewRef.current) observer.observe(previewRef.current);
    return () => {
      scroller.removeEventListener("scroll", check);
      observer.disconnect();
    };
  }, [previewKey, docBody]);

  // Markierung des Blocks unter dem Mauszeiger (zeigt, was der Stift bearbeitet).
  useEffect(() => {
    const block = hoverPen?.block;
    if (!block) return;
    block.setAttribute("data-edit-hover", "");
    return () => block.removeAttribute("data-edit-hover");
  }, [hoverPen?.block]);

  const onPreviewHover = (event: React.MouseEvent<HTMLDivElement>) => {
    const block = blockAt(event.target);
    if (!block) return;
    // Überschriften haben ihren eigenen Knopf.
    if (block.matches(SECTION_HEADINGS)) {
      if (hoverPen) setHoverPen(null);
      return;
    }
    // Innerster Block: ein Listenpunkt mit Absatz zeigt den Stift am Absatz.
    const wrapper = event.currentTarget.getBoundingClientRect();
    const rect = block.getBoundingClientRect();
    const top = rect.top - wrapper.top;
    if (hoverPen?.block === block && hoverPen.top === top) return;
    setHoverPen({ top, block });
  };

  /**
   * Two flavours in one clipboard write: `text/html` so a paste into Word,
   * Outlook or a mail client keeps headings and lists, and `text/plain` with
   * the Markdown source for editors that want it raw. The receiving
   * application picks whichever it understands.
   */
  const copyMinutes = async () => {
    if (!doc) return;
    setError(null);
    // D5: die Folienmarken sind im Dokument Schaltflaechen; in der Zwischenablage stehen sie
    // wieder als Text (`[F7]`), wie im gespeicherten Protokoll.
    const preview = previewRef.current?.cloneNode(true) as
      HTMLElement | undefined;
    preview
      ?.querySelectorAll("[data-edit-hover]")
      .forEach((el) => el.removeAttribute("data-edit-hover"));
    preview?.querySelectorAll("[data-slide-ref]").forEach((el) => {
      el.replaceWith(
        document.createTextNode(`[F${el.getAttribute("data-slide-ref")}]`),
      );
    });
    const html = preview?.innerHTML;
    try {
      if (html && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([
          new ClipboardItem({
            "text/html": new Blob([html], { type: "text/html" }),
            "text/plain": new Blob([doc.body], { type: "text/plain" }),
          }),
        ]);
      } else {
        // A webview without ClipboardItem still gets the Markdown.
        await navigator.clipboard.writeText(doc.body);
      }
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      setError(t("meetings.minutes.copyError") + `: ${String(e)}`);
    }
  };

  const exportMinutes = async () => {
    if (!doc) return;
    setError(null);
    setSaved(null);
    const baseName = docType
      ? `${meetingTitle} \u2013 ${docType.title}`
      : meetingTitle;
    const safeName =
      baseName.replace(/[\\/:*?"<>|]/g, "_").trim() || "protokoll";
    // Word zuerst: das Protokoll geht weiter an Menschen, nicht an einen
    // Editor. Das Format ergibt sich aus der gewählten Endung — deshalb
    // braucht es kein zweites Auswahlfeld neben dem Dialog.
    // Etappe 2/3: Word, PDF und PowerPoint mit dem eingestellten Briefkopf (Einstellungen > Ausgabe >
    // Dokumente); Quelle ist das gespeicherte Dokument samt Korrekturen.
    const target = await save({
      filters: [
        { name: "Word", extensions: ["docx"] },
        { name: "PDF", extensions: ["pdf"] },
        // Etappe 3: Folien aus dem Dokument (Titel, je Abschnitt Kernaussagen, Tabellen).
        { name: "PowerPoint", extensions: ["pptx"] },
        { name: "Text", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
      defaultPath: `${safeName}.docx`,
    });
    if (typeof target !== "string") return;
    // Geschrieben wird im Backend: das fs-Plugin lässt nur $APPDATA zu und
    // scheiterte an jedem Ziel, das der Dialog anbietet ("not allowed by ACL").
    const result = await commands.meetingsExportDocumentAs(doc.id, target);
    if (result.status !== "ok") {
      setError(t("meetings.minutes.exportError") + `: ${result.error}`);
      return;
    }
    setSaved(target);
  };

  if (loading) {
    return (
      <p className="text-sm text-text/60 text-center py-3">
        {t("meetings.list.loading")}
      </p>
    );
  }

  return (
    <div className="space-y-2" data-testid="minutes-view">
      {/* Schmale Werkzeugzeile statt Vorlagenwahl und Textknoepfen: der Inhalt
          beginnt direkt unter den Reitern. Vorlage waehlen, Vorlagen verwalten und
          "Neu erzeugen mit Vorlage" stehen im Menue neben "Details". */}
      <div
        role="toolbar"
        aria-label={t("meetings.minutes.toolbar")}
        data-testid="minutes-toolbar"
        className="minutes-toolbar sticky top-0 z-10 flex flex-wrap items-center gap-2 bg-background py-0.5"
      >
        {toolbarLead}
        <IconAction
          size="sm"
          icon={doc ? RefreshCw : Sparkles}
          label={
            doc
              ? t("meetings.detail.regenerate")
              : t("meetings.detail.generate")
          }
          description={t("meetings.minutes.regenerateHint")}
          testId="minutes-generate"
          disabled={running || editing !== null}
          onClick={() => void generate()}
        />
        {doc && (
          <>
            <IconAction
              size="sm"
              icon={Pencil}
              label={t("meetings.minutes.edit.start")}
              description={t("meetings.minutes.edit.startHint")}
              testId="minutes-edit"
              aria-pressed={editing !== null}
              className={
                editing
                  ? "!border-logo-primary/60 !bg-logo-primary/15 !text-text"
                  : ""
              }
              disabled={running || editing !== null}
              onClick={() => startEdit()}
            />
            {versions.length > 1 && (
              <ActionMenu
                menuLabel={t("meetings.minutes.versions.menu")}
                widthClass="w-72"
                trigger={{
                  icon: History,
                  size: "sm",
                  label: t("meetings.minutes.versions.label", {
                    version: doc.version,
                    count: versions.length,
                  }),
                  description: t("meetings.minutes.versions.hint"),
                  testId: "minutes-versions",
                  disabled: editing !== null,
                }}
                items={versions.map((v, i) => ({
                  id: v.id,
                  label: t(
                    v.edited
                      ? "meetings.minutes.versions.itemEdited"
                      : "meetings.minutes.versions.item",
                    {
                      version: v.version,
                      date: new Date(v.created_at * 1000).toLocaleString(
                        i18n.language,
                        { dateStyle: "short", timeStyle: "short" },
                      ),
                    },
                  ),
                  title:
                    i === 0 ? t("meetings.minutes.versions.latest") : undefined,
                  icon: v.id === doc.id ? Check : FileText,
                  testId: `minutes-version-${v.version}`,
                  onSelect: () => setSelectedId(i === 0 ? null : v.id),
                }))}
              />
            )}
            <IconAction
              size="sm"
              icon={copied ? Check : Copy}
              label={
                copied
                  ? t("meetings.detail.copied")
                  : docType
                    ? t("meetings.documents.copy")
                    : t("meetings.minutes.copy")
              }
              description={t("meetings.minutes.copyHint")}
              testId="minutes-copy"
              disabled={editing !== null}
              onClick={copyMinutes}
            />
            <IconAction
              size="sm"
              icon={Download}
              label={t("meetings.minutes.download")}
              description={t("meetings.detail.export")}
              testId="minutes-export"
              disabled={editing !== null}
              onClick={exportMinutes}
            />
            <ReadAloudButton
              speakKey={`minutes:${meetingId}:${docKey ?? "minutes"}`}
              getMarkdown={() => doc.body}
              testId="minutes-readaloud"
              disabled={editing !== null}
              fileStem={
                docType
                  ? `${meetingTitle} \u2013 ${docType.title}`
                  : meetingTitle
              }
            />
          </>
        )}
        {running && (
          <Badge variant="secondary">
            <span data-testid="minutes-running">
              {docType
                ? t("meetings.documents.generating", { title: docType.title })
                : t("meetings.minutes.generating")}
            </span>
          </Badge>
        )}
        {doc?.edited && (
          <Badge variant="secondary">
            <span data-testid="minutes-edited">
              {t("meetings.minutes.edit.edited")}
            </span>
          </Badge>
        )}
        {saved && (
          <span className="min-w-0 break-all text-xs text-text/60">
            {t("meetings.minutes.exportSaved", { path: saved })}
          </span>
        )}
      </div>

      {error && <Alert variant="error">{error}</Alert>}

      {running && <MinutesProgressBar progress={progress} />}

      {!running && meta?.incomplete && (
        <Alert variant="warning">
          <span data-testid="minutes-incomplete">
            {t("meetings.minutes.incomplete", {
              gaps: meta.gaps.length > 0 ? meta.gaps.join(", ") : "?",
            })}
          </span>
        </Alert>
      )}

      {doc && versions[0] && doc.id !== versions[0].id && (
        <Alert variant="info">
          <div className="flex flex-wrap items-center gap-2">
            <span className="me-auto" data-testid="minutes-old-version">
              {t("meetings.minutes.versions.olderShown", {
                version: doc.version,
                latest: versions[0].version,
              })}
            </span>
            <button
              type="button"
              className="min-h-8 rounded-md px-2 text-sm font-medium text-logo-primary hover:underline"
              data-testid="minutes-version-latest"
              onClick={() => setSelectedId(null)}
            >
              {t("meetings.minutes.versions.toLatest")}
            </button>
          </div>
        </Alert>
      )}

      {doc && editing ? (
        <MinutesEditor
          key={doc.id}
          doc={doc}
          reveal={editing.reveal}
          onClose={closeEditor}
          onSaved={(next) => {
            setDoc(next);
            closeEditor();
          }}
        />
      ) : doc ? (
        <div
          ref={wrapperRef}
          className="minutes-preview relative"
          onMouseOver={onPreviewHover}
          onMouseLeave={() => setHoverPen(null)}
          onDoubleClick={(event) => {
            const block = blockAt(event.target);
            if (block) startEdit(block);
          }}
        >
          <div
            ref={previewRef}
            data-testid="minutes-doc"
            className="rounded-lg border border-mid-gray/20 p-4 pe-10"
          >
            <DocUsageLine documentId={doc.id} />
            <MarkdownContent markdown={doc.body} slideRefs={slideRefs} />
          </div>
          {!running &&
            sections.map((section, i) => (
              <button
                key={`${i}:${section.title}`}
                type="button"
                data-testid="minutes-section-edit"
                title={t("meetings.minutes.edit.sectionHint", {
                  title: section.title,
                })}
                aria-label={t("meetings.minutes.edit.sectionLabel", {
                  title: section.title,
                })}
                onClick={() => startEdit(section.el)}
                style={{ top: section.top }}
                className="minutes-section-pen absolute end-2 flex h-8 items-center gap-1.5 rounded-md border border-mid-gray/30 bg-background px-2 text-xs font-medium text-text/80 hover:border-logo-primary hover:text-logo-primary focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
              >
                <Pencil width={14} height={14} aria-hidden="true" />
                <span className="minutes-section-pen__text">
                  {t("meetings.minutes.edit.section")}
                </span>
              </button>
            ))}
          {hoverPen && !running && (
            <button
              type="button"
              data-testid="minutes-block-edit"
              title={t("meetings.minutes.edit.block")}
              aria-label={t("meetings.minutes.edit.block")}
              onClick={() => startEdit(hoverPen.block)}
              style={{ top: hoverPen.top }}
              className="minutes-block-pen absolute end-2 flex h-7 w-7 items-center justify-center rounded-md border border-logo-primary/50 bg-background text-logo-primary hover:bg-logo-primary/10 focus:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              <Pencil width={14} height={14} aria-hidden="true" />
            </button>
          )}
          {moreBelow && (
            <div
              aria-hidden="true"
              data-testid="minutes-more-below"
              className="pointer-events-none sticky bottom-0 -mt-12 flex h-12 items-end justify-center bg-gradient-to-t from-background via-background/80 to-transparent pb-1"
            >
              <span className="flex items-center gap-1 rounded-full border border-mid-gray/30 bg-background px-2.5 py-0.5 text-xs text-text/80 shadow-sm">
                <ChevronDown width={14} height={14} />
                {t("meetings.minutes.moreBelow")}
              </span>
            </div>
          )}
        </div>
      ) : (
        !running && (
          <p className="text-sm text-text/60" data-testid="minutes-empty">
            {docType
              ? t("meetings.documents.empty", {
                  title: docType.title,
                  audience: docType.audience,
                  length: docType.length,
                })
              : t("meetings.minutes.empty")}
          </p>
        )
      )}
      {guard.dialog}
    </div>
  );
};

/**
 * Fortschrittsbalken der Protokoll-Erzeugung: Phase, Schritt und Prozent. Ohne
 * bekannte Gesamtzahl (Vorlagenwahl, erster Augenblick) laeuft er unbestimmt.
 */
const MinutesProgressBar: React.FC<{ progress: MinutesProgress | null }> = ({
  progress,
}) => {
  const { t } = useTranslation();
  const percent = progressPercent(progress);
  return (
    <div className="space-y-1" data-testid="minutes-progress">
      <div className="flex items-center justify-between text-xs text-text/60">
        <span data-testid="minutes-phase">
          {progress
            ? t(phaseKey(progress.phase))
            : t("meetings.minutes.generating")}
        </span>
        <span data-testid="minutes-percent">
          {percent !== null && progress
            ? `${t("meetings.minutes.stepOf", { done: progress.done, total: progress.total })} \u00b7 ${percent} %`
            : ""}
        </span>
      </div>
      <div
        role="progressbar"
        aria-label={t("meetings.minutes.progressLabel")}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-1.5 w-full overflow-hidden rounded-full bg-mid-gray/20"
      >
        <div
          className={`h-full rounded-full bg-logo-primary transition-[width] duration-300 ${
            percent === null ? "w-1/3 animate-pulse" : ""
          }`}
          style={percent === null ? undefined : { width: `${percent}%` }}
        />
      </div>
    </div>
  );
};
