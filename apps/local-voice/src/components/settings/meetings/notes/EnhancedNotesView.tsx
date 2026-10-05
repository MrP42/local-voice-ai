import { defaultDocBasis } from "../language/DocBasisFields";
import { documentBasisText, useDocumentBasis } from "../language/documentBasis";
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { save } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  Download,
  Plus,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import {
  commands,
  events,
  type ActionItem,
  type EnhancedEntry,
  type EnhancedNotes,
  type EnhancedSection,
  type Meeting,
  type MeetingDocument,
  type StoredSegment,
} from "@/bindings";
import {
  AUTOSAVE_DEBOUNCE_MS,
  ENHANCED_KIND,
  enhanceErrorCode,
  enhanceErrorText,
  entryIds,
  exportFileName,
  hasEmptyEntry,
  isAiText,
  isStale,
  lacksEvidence,
  needsProviderSetup,
  parseEnhanced,
  withEntryText,
  withNewEntry,
  withoutEmptyEntries,
} from "@/lib/meetingNotes";
import { useJobEnded, useMeetingProgress } from "@/hooks/useMeetingJobs";
import { Alert } from "../../../ui/Alert";
import { Dropdown } from "../../../ui/Dropdown";
import { IconAction } from "../../../ui/IconAction";
import { InstructionBar } from "./InstructionBar";
import { SourceChips } from "./SourceChips";
import { TaskChecklist } from "./TaskChecklist";

interface EnhancedNotesViewProps {
  meeting: Meeting;
  /** Segmente des Transkripts: liefern die Zeit hinter jeder Quellnummer. */
  segments: StoredSegment[];
  /** Zaehler, der sich mit jeder moeglichen Neu-Transkription erhoeht (Epoche neu lesen). */
  epochKey: number;
  /** Ist noch Audio vorhanden, spielt der Sprung ab; sonst geht er nur ins Transkript. */
  hasAudio: boolean;
  /** Wechselt ins Transkript, markiert das Segment und spielt (falls Audio) ab. */
  onJumpToSource: (segmentIndex: number) => void;
  /** D5: erste Sichtung einer Folie in ms (`null`: es gibt sie nicht (mehr)). */
  slideStartMsOf?: (slideNumber: number) => number | null;
  /** D5: Sprung zu einer Folie, auf die sich ein Eintrag belegt (`F7`). */
  onJumpToSlide?: (slideNumber: number) => void;
}

type SaveStatus = "idle" | "dirty" | "saving" | "saved" | "error";

/** Eintragstext: anklickbar zum Bearbeiten, dabei ein auto-wachsendes Feld. */
const EntryText: React.FC<{
  entry: EnhancedEntry;
  done: boolean;
  readOnly: boolean;
  editing: boolean;
  onStart: () => void;
  onChange: (text: string) => void;
  onFinish: () => void;
}> = ({ entry, done, readOnly, editing, onStart, onChange, onFinish }) => {
  const { t } = useTranslation();
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const ai = isAiText(entry);
  const tone = done
    ? "text-text/50 line-through"
    : ai
      ? "text-text/60"
      : "text-text";

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [entry.text, editing]);

  useEffect(() => {
    if (!editing) return;
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, [editing]);

  if (editing) {
    return (
      <textarea
        ref={ref}
        rows={1}
        value={entry.text}
        data-testid="entry-editor"
        aria-label={t("meetings.enhanced.editEntry")}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onFinish}
        onKeyDown={(e) => {
          // Enter und Escape beenden das Bearbeiten (Shift+Enter: Zeilenumbruch).
          if ((e.key === "Enter" && !e.shiftKey) || e.key === "Escape") {
            e.preventDefault();
            e.currentTarget.blur();
          }
        }}
        className={`w-full resize-none overflow-hidden rounded-sm bg-mid-gray/10 px-1 py-0 text-sm leading-6 focus:outline-none focus:ring-1 focus:ring-logo-primary ${tone}`}
      />
    );
  }
  if (readOnly) {
    return (
      <p
        className={`whitespace-pre-wrap break-words text-sm leading-6 ${tone}`}
      >
        {entry.text}
      </p>
    );
  }
  return (
    <button
      type="button"
      data-testid="entry-text"
      title={t("meetings.enhanced.editHint")}
      onClick={onStart}
      className={`w-full cursor-text whitespace-pre-wrap break-words rounded-sm px-1 text-left text-sm leading-6 hover:bg-mid-gray/10 focus:outline-none focus-visible:ring-1 focus-visible:ring-logo-primary ${tone}`}
    >
      {entry.text}
    </button>
  );
};

export const EnhancedNotesView: React.FC<EnhancedNotesViewProps> = ({
  meeting,
  segments,
  epochKey,
  hasAudio,
  onJumpToSource,
  slideStartMsOf,
  onJumpToSlide,
}) => {
  const { t, i18n } = useTranslation();
  const meetingId = meeting.id;

  const [docs, setDocs] = useState<MeetingDocument[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [notes, setNotes] = useState<EnhancedNotes | null>(null);
  const [items, setItems] = useState<ActionItem[]>([]);
  const [epoch, setEpoch] = useState<number | null>(null);
  const [status, setStatus] = useState<SaveStatus>("idle");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [addingIn, setAddingIn] = useState<string | null>(null);
  const [addText, setAddText] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{
    step: number;
    total: number;
  } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [reloaded, setReloaded] = useState(false);
  const [exported, setExported] = useState<string | null>(null);
  // P8a: der Lauf steht im Backend (Auftrag "KI-Notizen"): beim Reiterwechsel
  // erscheint die Ansicht neu und sieht trotzdem, dass er noch laeuft.
  const progressMap = useMeetingProgress();
  const notesJob =
    progressMap[meetingId]?.phase === "notes"
      ? progressMap[meetingId]
      : undefined;
  const [stoppedNote, setStoppedNote] = useState(false);

  // Stand des Editors ausserhalb von React: der entprellte Speichervorgang
  // liest immer die neueste Fassung, nie eine veraltete Closure.
  const notesRef = useRef<EnhancedNotes | null>(null);
  const docIdRef = useRef<string | null>(null);
  const stampRef = useRef(0);
  const serverIdsRef = useRef<string[]>([]);
  const dirtyRef = useRef(false);
  const selectedIdRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const inflightRef = useRef<Promise<void> | null>(null);
  const mountedRef = useRef(true);
  const newEntryCounter = useRef(0);

  const safe = useCallback((fn: () => void) => {
    if (mountedRef.current) fn();
  }, []);

  const startMs = useMemo(() => {
    const map = new Map(segments.map((s) => [s.segment_index, s.start_ms]));
    return (index: number) => map.get(index) ?? null;
  }, [segments]);

  const selected = docs.find((d) => d.id === selectedId) ?? docs[0] ?? null;
  const readOnly = selected !== null && docs[0]?.id !== selected.id;
  // G5: aus welcher Fassung des Transkripts und in welcher Sprache diese Version entstand.
  const basis = useDocumentBasis(
    meetingId,
    "enhanced_notes",
    selected?.id ?? null,
    selected?.id ?? "",
  );
  const stale = notes !== null && isStale(notes, epoch);
  const running = busy || progress !== null || notesJob !== undefined;

  // ---- Laden ------------------------------------------------------------

  const applyDoc = useCallback((doc: MeetingDocument | null) => {
    const parsed = doc ? parseEnhanced(doc.body) : null;
    notesRef.current = parsed;
    docIdRef.current = doc?.id ?? null;
    stampRef.current = doc?.updated_at ?? 0;
    serverIdsRef.current = parsed ? entryIds(parsed) : [];
    dirtyRef.current = false;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    setNotes(parsed);
    setEditingId(null);
    setStatus("idle");
  }, []);

  const loadItems = useCallback(async () => {
    const result = await commands.actionItemsList(meetingId);
    if (result.status === "ok") safe(() => setItems(result.data));
  }, [meetingId, safe]);

  /**
   * Dokumente neu lesen. Ungespeicherte Eingaben in der angezeigten Version
   * bleiben stehen (`force` erzwingt den Serverstand, z. B. nach `stale_document`).
   */
  const loadDocs = useCallback(
    async (force = false) => {
      const result = await commands.meetingsGetDocuments(meetingId);
      if (!mountedRef.current) return;
      setLoaded(true);
      if (result.status !== "ok") {
        setLoadError(result.error);
        return;
      }
      setLoadError(null);
      const list = result.data
        .filter((d) => d.kind === ENHANCED_KIND)
        .sort((a, b) => b.version - a.version);
      setDocs(list);
      const doc =
        list.find((d) => d.id === selectedIdRef.current) ?? list[0] ?? null;
      const keepLocal =
        dirtyRef.current && !force && doc?.id === docIdRef.current;
      if (!keepLocal) applyDoc(doc);
    },
    [meetingId, applyDoc],
  );

  useEffect(() => {
    mountedRef.current = true;
    selectedIdRef.current = null;
    setSelectedId(null);
    setLoaded(false);
    setError(null);
    void loadDocs(true);
    void loadItems();
    return () => {
      mountedRef.current = false;
    };
    // Nur beim Wechsel der Besprechung neu laden.
  }, [meetingId]);

  useEffect(() => {
    let cancelled = false;
    void commands.meetingsSegmentEpoch(meetingId).then((result) => {
      if (!cancelled && result.status === "ok") setEpoch(result.data);
    });
    return () => {
      cancelled = true;
    };
  }, [meetingId, epochKey]);

  // Fortschritt, Ende und Fehler des Laufs (auch des automatischen nach dem Stopp).
  useEffect(() => {
    const un = events.meetingNotesEvent.listen((event) => {
      const payload = event.payload;
      if (payload.meeting_id !== meetingId) return;
      if (payload.kind === "progress") {
        setStoppedNote(false);
        setProgress({ step: payload.step, total: payload.total });
      } else if (payload.kind === "done") {
        setProgress(null);
        setError(null);
        selectedIdRef.current = null;
        setSelectedId(null);
        void loadDocs();
        void loadItems();
      } else if (payload.code === "stopped") {
        // P8a: vom Nutzer gestoppt ist keine Panne.
        setProgress(null);
        setError(null);
        setStoppedNote(true);
      } else {
        setProgress(null);
        setError(payload.code);
      }
    });
    return () => {
      un.then((f) => f());
    };
  }, [meetingId, loadDocs, loadItems]);

  // P8a: das Ende des Auftrags laedt das Ergebnis neu, auch wenn die Ansicht
  // beim Ende nicht offen war (Reiterwechsel).
  useJobEnded(meetingId, (ended) => {
    if (ended.phase !== "notes") return;
    setProgress(null);
    void loadDocs();
    void loadItems();
  });

  // ---- Speichern (entprellt, ein Vorgang zugleich) -----------------------

  const saveOnce = useCallback(async (): Promise<void> => {
    const docId = docIdRef.current;
    const snapshot = notesRef.current;
    if (!docId || !snapshot || !dirtyRef.current) return;
    if (hasEmptyEntry(snapshot)) return; // erst beim Verlassen des Feldes bereinigt
    safe(() => setStatus("saving"));
    const result = await commands.meetingNotesUpdateEnhanced(
      docId,
      snapshot,
      stampRef.current,
    );
    if (docIdRef.current !== docId) return; // Version wurde gewechselt
    if (result.status === "ok") {
      stampRef.current = result.data;
      const structural =
        entryIds(snapshot).join("|") !== serverIdsRef.current.join("|");
      if (notesRef.current === snapshot) {
        dirtyRef.current = false;
        safe(() => setStatus("saved"));
      } else {
        safe(() => setStatus("dirty"));
      }
      // Die Aufgaben entstehen beim Speichern neu (neue IDs), neue Eintraege
      // bekommen ihre endgueltige ID vom Backend: beides frisch lesen.
      void loadItems();
      if (structural && notesRef.current === snapshot) await loadDocs(true);
      else serverIdsRef.current = entryIds(snapshot);
      return;
    }
    if (enhanceErrorCode(result.error) === "stale_document") {
      // Zweites Fenster oder neuer Lauf war schneller: Serverstand laden, Hinweis zeigen.
      safe(() => setReloaded(true));
      await loadDocs(true);
      void loadItems();
      return;
    }
    safe(() => {
      setStatus("error");
      setError(result.error);
    });
  }, [loadDocs, loadItems, safe]);

  const flush = useCallback(async (): Promise<void> => {
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    for (let round = 0; round < 3; round++) {
      if (inflightRef.current) await inflightRef.current;
      if (!dirtyRef.current) return;
      const before = notesRef.current;
      const run = saveOnce();
      inflightRef.current = run;
      await run;
      if (inflightRef.current === run) inflightRef.current = null;
      if (notesRef.current === before) return;
    }
  }, [saveOnce]);

  const schedule = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void flush();
    }, AUTOSAVE_DEBOUNCE_MS);
  }, [flush]);

  // Ausblenden (anderer Tab, Ansicht, Besprechung): Ungespeichertes wegschreiben.
  useEffect(
    () => () => {
      void flush();
    },
    [flush],
  );

  const change = useCallback(
    (next: EnhancedNotes) => {
      notesRef.current = next;
      dirtyRef.current = true;
      setNotes(next);
      setStatus("dirty");
      schedule();
    },
    [schedule],
  );

  const editEntry = (entryId: string, text: string) => {
    if (!notesRef.current) return;
    change(withEntryText(notesRef.current, entryId, text));
  };

  /** Feld verlassen: leere Eintraege entfallen, dann sofort speichern. */
  const finishEdit = () => {
    setEditingId(null);
    const current = notesRef.current;
    if (current && hasEmptyEntry(current)) change(withoutEmptyEntries(current));
    void flush();
  };

  const finishAdd = (sectionId: string) => {
    const text = addText.trim();
    setAddingIn(null);
    setAddText("");
    const current = notesRef.current;
    if (!current || text === "") return;
    newEntryCounter.current += 1;
    change(
      withNewEntry(current, sectionId, `new-${newEntryCounter.current}`, text),
    );
    void flush();
  };

  // ---- Erzeugen, Anweisung, Export -----------------------------------------

  const generate = async () => {
    await flush();
    setBusy(true);
    setError(null);
    setStoppedNote(false);
    setReloaded(false);
    // `None`: die Vorlage, die fuer diese Besprechung gewaehlt ist.
    // G5: aktive Fassung, Sprache = letzte Wahl bzw. die der App.
    const result = await commands.meetingNotesEnhance(
      meetingId,
      null,
      defaultDocBasis(i18n.language),
    );
    safe(() => setBusy(false));
    if (result.status === "error") {
      // Gestoppt (P8a) meldet das Ereignis; hier keine Fehlermeldung daraus machen.
      if (result.error === "stopped") {
        safe(() => setStoppedNote(true));
        return;
      }
      safe(() => setError(result.error));
      return;
    }
    selectedIdRef.current = null;
    safe(() => setSelectedId(null));
    await loadDocs(true);
    void loadItems();
  };

  const applyInstruction = async (instruction: string): Promise<boolean> => {
    if (!selected) return false;
    await flush();
    setBusy(true);
    setError(null);
    setReloaded(false);
    const result = await commands.meetingNotesApplyInstruction(
      selected.id,
      instruction,
    );
    safe(() => setBusy(false));
    if (result.status === "error") {
      safe(() => setError(result.error));
      return false;
    }
    selectedIdRef.current = null;
    safe(() => setSelectedId(null));
    await loadDocs(true);
    void loadItems();
    return true;
  };

  const exportMarkdown = async () => {
    if (!selected) return;
    await flush();
    setExported(null);
    const target = await save({
      defaultPath: `${exportFileName(meeting.title)} - ${t("meetings.enhanced.exportSuffix")}.docx`,
      filters: [
        { name: "Word", extensions: ["docx"] },
        { name: "Text", extensions: ["txt"] },
        { name: "Markdown", extensions: ["md"] },
      ],
    });
    if (typeof target !== "string") return;
    const markdown = await commands.meetingNotesMarkdown(selected.id);
    if (markdown.status === "error") {
      setError(markdown.error);
      return;
    }
    const written = await commands.meetingsExportDocument(
      target,
      markdown.data,
    );
    if (written.status === "error") {
      setError(written.error);
      return;
    }
    setExported(target);
  };

  const toggleTask = async (item: ActionItem, done: boolean) => {
    const status = done ? "done" : "todo";
    setItems((prev) =>
      prev.map((i) => (i.id === item.id ? { ...i, status } : i)),
    );
    const result = await commands.actionItemsSetStatus(item.id, done);
    if (result.status === "error") {
      setItems((prev) =>
        prev.map((i) => (i.id === item.id ? { ...i, status: item.status } : i)),
      );
      setError(result.error);
    }
  };

  const selectVersion = async (id: string) => {
    await flush();
    selectedIdRef.current = id;
    setSelectedId(id);
    applyDoc(docs.find((d) => d.id === id) ?? null);
  };

  // ---- Darstellung ---------------------------------------------------------

  const itemOf = (entry: EnhancedEntry): ActionItem | undefined =>
    selected
      ? items.find(
          (i) => i.document_id === selected.id && i.entry_id === entry.id,
        )
      : undefined;

  const dateFormat = new Intl.DateTimeFormat(i18n.language, {
    dateStyle: "short",
    timeStyle: "short",
  });
  const versionOptions = docs.map((d) => ({
    value: d.id,
    label: t("meetings.enhanced.versionLabel", {
      version: d.version,
      date: dateFormat.format(new Date(d.created_at * 1000)),
    }),
  }));

  const renderEntry = (entry: EnhancedEntry, done: boolean) => {
    const ai = isAiText(entry);
    const noEvidence = lacksEvidence(entry);
    return (
      <div
        data-testid="enhanced-entry"
        data-entry-id={entry.id}
        data-origin={ai ? "ai" : "user"}
        // Umbrechend: in der schmalen Bedienspalte nahmen die Belegchips dem
        // Text sonst fast die ganze Breite (ein Wort je Zeile). Unter 10rem
        // Textbreite rutschen sie rechtsbuendig unter den Text.
        className="flex flex-wrap items-start gap-x-2 gap-y-0.5"
      >
        <div className="min-w-[10rem] flex-1">
          <EntryText
            entry={entry}
            done={done}
            readOnly={readOnly}
            editing={editingId === entry.id}
            onStart={() => setEditingId(entry.id)}
            onChange={(text) => editEntry(entry.id, text)}
            onFinish={finishEdit}
          />
        </div>
        <div className="ml-auto flex max-w-full shrink-0 items-center pt-0.5">
          {noEvidence ? (
            <span
              data-testid="no-evidence"
              title={t("meetings.enhanced.noEvidenceHint")}
              className="inline-flex items-center gap-1 rounded-md bg-yellow-500/10 px-1.5 py-0.5 text-[11px] leading-none text-yellow-500"
            >
              <AlertTriangle width={11} height={11} aria-hidden="true" />
              {t("meetings.enhanced.noEvidence")}
            </span>
          ) : (
            <SourceChips
              ids={entry.source_segment_ids}
              startMsOf={startMs}
              stale={stale}
              onJump={onJumpToSource}
              slideIds={entry.source_slide_ids}
              slideStartMsOf={slideStartMsOf}
              onJumpSlide={onJumpToSlide}
            />
          )}
        </div>
      </div>
    );
  };

  const renderSection = (section: EnhancedSection) => (
    <section
      key={section.id}
      data-section-id={section.id}
      data-kind={section.kind}
      className="space-y-1"
    >
      <h4 className="text-sm font-semibold text-text">{section.title}</h4>
      {section.entries.length === 0 && addingIn !== section.id && (
        <p className="text-xs text-text/40">
          {t("meetings.enhanced.sectionEmpty")}
        </p>
      )}
      {section.kind === "tasks" ? (
        <TaskChecklist
          section={section}
          itemOf={itemOf}
          onToggle={(item, done) => void toggleTask(item, done)}
          readOnly={readOnly}
          renderEntry={renderEntry}
        />
      ) : (
        <ul className="space-y-1">
          {section.entries.map((entry) => (
            <li key={entry.id} className="flex items-start gap-2">
              <span
                aria-hidden="true"
                className="w-3 shrink-0 select-none text-center text-sm leading-6 text-text/40"
              >
                •
              </span>
              <div className="min-w-0 flex-1">{renderEntry(entry, false)}</div>
            </li>
          ))}
        </ul>
      )}
      {addingIn === section.id && (
        <textarea
          rows={1}
          value={addText}
          autoFocus
          data-testid="new-entry"
          aria-label={t("meetings.enhanced.newEntry")}
          placeholder={t("meetings.enhanced.newEntry")}
          onChange={(e) => setAddText(e.target.value)}
          onBlur={() => finishAdd(section.id)}
          onKeyDown={(e) => {
            if ((e.key === "Enter" && !e.shiftKey) || e.key === "Escape") {
              e.preventDefault();
              e.currentTarget.blur();
            }
          }}
          className="ml-5 w-[calc(100%-1.25rem)] resize-none rounded-sm bg-mid-gray/10 px-1 text-sm leading-6 text-text focus:outline-none focus:ring-1 focus:ring-logo-primary"
        />
      )}
      {!readOnly && addingIn !== section.id && (
        <button
          type="button"
          onClick={() => {
            setAddText("");
            setAddingIn(section.id);
          }}
          className="ml-5 inline-flex cursor-pointer items-center gap-1 text-xs text-text/40 hover:text-logo-primary focus:outline-none focus-visible:text-logo-primary"
        >
          <Plus width={12} height={12} aria-hidden="true" />
          {t("meetings.enhanced.addEntry")}
        </button>
      )}
    </section>
  );

  const errorText = error ? enhanceErrorText(error) : null;
  const errorCode = error ? (enhanceErrorCode(error) ?? "") : "";
  const hasSources =
    notes?.sections.some((s) =>
      s.entries.some((e) => e.source_segment_ids.length > 0),
    ) ?? false;
  const canGenerate = meeting.status === "ready" && !running;
  const statusLabel =
    status === "saving"
      ? t("meetings.notes.status.saving")
      : status === "dirty"
        ? t("meetings.notes.status.dirty")
        : status === "saved"
          ? t("meetings.notes.status.saved")
          : "";

  return (
    <div
      className="space-y-3"
      data-testid="enhanced-notes"
      data-stale={stale ? "true" : "false"}
    >
      {/* Schmale Werkzeugzeile: Neu erzeugen und Herunterladen als Symbole,
          bei mehreren Versionen die Auswahl daneben. Vorlage und Herkunft stehen
          im Kopf (Chip) und im Details-Dialog, die Wahl der Vorlage im Menue. */}
      <div
        role="toolbar"
        aria-label={t("meetings.enhanced.toolbar")}
        data-testid="enhanced-toolbar"
        className="flex flex-wrap items-center gap-2"
      >
        <IconAction
          size="sm"
          icon={docs.length > 0 ? RefreshCw : Sparkles}
          label={
            docs.length > 0
              ? t("meetings.enhanced.regenerate")
              : t("meetings.enhanced.generate")
          }
          description={
            meeting.status !== "ready"
              ? t("meetings.enhanced.errors.meeting_not_finished")
              : t("meetings.enhanced.regenerateHint")
          }
          testId="enhanced-generate"
          disabled={!canGenerate}
          onClick={() => void generate()}
        />
        {selected && (
          <IconAction
            size="sm"
            icon={Download}
            label={t("meetings.minutes.download")}
            description={t("meetings.enhanced.export")}
            testId="enhanced-export"
            onClick={() => void exportMarkdown()}
          />
        )}
        {docs.length > 1 && (
          <div data-testid="version-picker" className="min-w-0">
            <Dropdown
              options={versionOptions}
              selectedValue={selected?.id ?? null}
              onSelect={(id) => void selectVersion(id)}
              className="min-w-[12rem]"
            />
          </div>
        )}
      </div>

      {basis && selected && (
        <p className="text-xs text-text/70" data-testid="enhanced-basis">
          {documentBasisText(basis, "enhanced_notes", t, i18n.language)}
        </p>
      )}

      {progress && !notesJob && (
        <p
          className="text-sm text-text/70"
          aria-live="polite"
          data-testid="enhance-progress"
        >
          {t("meetings.enhanced.progress", {
            step: progress.step,
            total: progress.total,
          })}
        </p>
      )}
      {(busy || notesJob) && !progress && (
        <p className="text-sm text-text/70" aria-live="polite">
          {t("meetings.enhanced.working")}
        </p>
      )}
      {stoppedNote && !running && (
        <div data-testid="enhance-stopped">
          <Alert variant="info">{t("meetings.enhanced.stopped")}</Alert>
        </div>
      )}
      {errorText && (
        <div data-testid="enhance-error" data-code={errorCode}>
          <Alert variant="error">
            {t(errorText.key, errorText.params)}
            {error && needsProviderSetup(error) && (
              <> {t("meetings.enhanced.providerHint")}</>
            )}
          </Alert>
        </div>
      )}
      {loadError && (
        <Alert variant="error">
          {t("meetings.enhanced.loadError", { error: loadError })}
        </Alert>
      )}
      {reloaded && (
        <Alert variant="info">{t("meetings.enhanced.reloaded")}</Alert>
      )}
      {stale && (
        <div data-testid="stale-hint">
          <Alert variant="warning">{t("meetings.enhanced.staleSources")}</Alert>
        </div>
      )}
      {!stale && notes && hasSources && !hasAudio && (
        <p className="text-xs text-text/60" data-testid="audio-gone-hint">
          {t("meetings.enhanced.audioGone")}
        </p>
      )}
      {readOnly && (
        <p className="text-xs text-text/60" data-testid="old-version-hint">
          {t("meetings.enhanced.oldVersion")}
        </p>
      )}
      {notes && notes.stats.chunks_failed.length > 0 && (
        <Alert variant="warning">
          {t("meetings.enhanced.chunksFailed", {
            failed: notes.stats.chunks_failed.length,
            total: notes.stats.chunks_total,
          })}
        </Alert>
      )}

      {!loaded ? (
        <p className="py-3 text-center text-sm text-text/60">
          {t("meetings.list.loading")}
        </p>
      ) : notes ? (
        <div className="space-y-4 rounded-lg border border-mid-gray/20 px-3 py-3">
          {notes.sections.map(renderSection)}
        </div>
      ) : selected ? (
        <Alert variant="error">{t("meetings.enhanced.unreadable")}</Alert>
      ) : (
        <p className="text-sm text-text/60" data-testid="ai-notes-placeholder">
          {t("meetings.enhanced.empty")}
        </p>
      )}

      {selected && notes && (
        <>
          <p
            aria-live="polite"
            data-testid="enhanced-status"
            data-status={status}
            className={`text-xs ${status === "error" ? "text-red-400" : "text-text/50"}`}
          >
            {statusLabel}
          </p>
          <InstructionBar
            onApply={applyInstruction}
            busy={running}
            disabled={readOnly}
          />
        </>
      )}
      {exported && (
        <p className="break-all text-xs text-text/60" data-testid="exported">
          {t("meetings.enhanced.exported", { path: exported })}
        </p>
      )}
    </div>
  );
};
