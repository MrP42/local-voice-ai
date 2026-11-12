import React, {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Bold,
  CaseSensitive,
  ChevronDown,
  ChevronUp,
  ClipboardCheck,
  Clock,
  Eraser,
  Heading1,
  Heading2,
  Heading3,
  Italic,
  List,
  ListChecks,
  Minus,
  Pilcrow,
  Quote,
  Regex,
  Replace,
  ReplaceAll,
  Search,
  SpellCheck,
  Sparkles,
  Unlink2,
  Wand2,
  WholeWord,
  WrapText,
  X,
  Space,
  type LucideIcon,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import { IconAction } from "../../../ui/IconAction";
import { ActionMenu } from "../../../ui/ActionMenu";
import { Button } from "../../../ui/Button";
import type { ChipEditorInsertApi } from "./TtsChipEditor";
import {
  applyFormat,
  findAll,
  replaceAll,
  replacementAt,
  type FindOptions,
  type FormatKind,
} from "@/lib/reader/editorOps";
import {
  applyCleanupInRange,
  CLEANUP_RULES,
  type CleanupRule,
} from "@/lib/reader/textCleanup";
import {
  applyGrammar,
  applyStructure,
  buildAssistInput,
  type AssistTask,
} from "@/lib/reader/textAssist";

interface EditorMenuProps {
  /** Text des aktiven Reiters (Original, Übersetzung, KI-Dokument). */
  value: string;
  /** Ganzen Text setzen (landet in der Seiten-Historie: Rückgängig geht). */
  onChange(next: string): void;
  /** Bearbeiten gesperrt (Übersetzung läuft gerade). */
  readOnly?: boolean;
  apiRef: React.RefObject<ChipEditorInsertApi | null>;
  spellCheck: boolean;
  onSpellCheck(on: boolean): void;
  /** Beim Einfügen automatisch bereinigen (nur der Original-Editor kennt
   *  den Schalter; ohne `onPasteCleanup` erscheint der Eintrag nicht). */
  pasteCleanup?: boolean;
  onPasteCleanup?(on: boolean): void;
  /** Bestehendes „Text aufbereiten" (ganzer Text über das LLM). */
  onTidy?: () => void;
  tidying?: boolean;
  /**
   * Suchen und Ersetzen mit sichtbarer Beschriftung statt nur Symbolen
   * (Protokoll-Editor): Knopf „Suchen und Ersetzen“ in der Leiste, Feldnamen
   * und Textknöpfe „Ersetzen“/„Alle ersetzen“ in der Suchleiste.
   */
  labeled?: boolean;
}

const FMT: { kind: FormatKind; icon: LucideIcon; key: string }[] = [
  { kind: "h1", icon: Heading1, key: "h1" },
  { kind: "h2", icon: Heading2, key: "h2" },
  { kind: "h3", icon: Heading3, key: "h3" },
  { kind: "paragraph", icon: Pilcrow, key: "paragraph" },
  { kind: "bold", icon: Bold, key: "bold" },
  { kind: "italic", icon: Italic, key: "italic" },
  { kind: "quote", icon: Quote, key: "quote" },
  { kind: "list", icon: List, key: "list" },
  { kind: "rule", icon: Minus, key: "rule" },
];

const RULE_ICONS: Record<CleanupRule, LucideIcon> = {
  timestamps: Clock,
  fragments: Unlink2,
  spaces: Space,
  paragraphs: WrapText,
};

const SEP = "mx-1 h-5 w-px shrink-0 bg-mid-gray/30";
const TOGGLE_ON = "!border-logo-primary/60 !bg-logo-primary/15 !text-text";

/**
 * Editor-Menü über dem Textfeld (Bearbeiten-Ansicht): Suchen und Ersetzen,
 * Rechtschreibprüfung, Formatierung für die Leseansicht, Bereinigen nach
 * Regeln (ohne KI, ohne Token) und KI-Textarbeit (Grammatik, Gliederung).
 *
 * Alles wirkt auf die Markierung, ohne Markierung auf den ganzen Text, und
 * jede Änderung ist ein Schritt in der Historie (Strg+Z).
 */
export const EditorMenu: React.FC<EditorMenuProps> = ({
  value,
  onChange,
  readOnly = false,
  apiRef,
  spellCheck,
  onSpellCheck,
  pasteCleanup = true,
  onPasteCleanup,
  onTidy,
  tidying = false,
  labeled = false,
}) => {
  const { t } = useTranslation();
  const [findOpen, setFindOpen] = useState(false);
  // Strg+F bei offener Leiste: Suchfeld erneut fokussieren.
  const [findFocus, setFindFocus] = useState(0);
  const [replaceOpen, setReplaceOpen] = useState(false);
  const [busy, setBusy] = useState<AssistTask | null>(null);
  const [progress, setProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  const empty = value.trim().length === 0;
  // Aktueller Text für Rückläufer aus der KI: wer währenddessen getippt hat,
  // verliert nichts (Ergebnis wird dann verworfen).
  const latest = useRef(value);
  latest.current = value;

  const selection = () =>
    apiRef.current?.getSelection?.() ?? { start: 0, end: 0 };

  // ---- Formatierung ------------------------------------------------------
  const format = (kind: FormatKind) => {
    const api = apiRef.current;
    if (!api || readOnly) return;
    const sel = selection();
    const edit = applyFormat(value, sel.start, sel.end, kind);
    if (api.replaceRange) api.replaceRange(edit.start, edit.end, edit.insert);
    else
      onChange(
        value.slice(0, edit.start) + edit.insert + value.slice(edit.end),
      );
    window.requestAnimationFrame(() =>
      api.revealRange?.(edit.selStart, edit.selEnd),
    );
  };

  // ---- Bereinigen (Regeln) -----------------------------------------------
  const cleanup = (rules: readonly CleanupRule[]) => {
    if (readOnly) return;
    const sel = selection();
    const result = applyCleanupInRange(value, rules, sel.start, sel.end);
    if (result.changes === 0 || result.text === value) {
      toast(t("tts.editorMenu.cleanupNothing"));
      return;
    }
    onChange(result.text);
    toast(t("tts.editorMenu.cleanupDone", { count: result.changes }));
  };

  // ---- KI ----------------------------------------------------------------
  useEffect(() => {
    if (!busy) return;
    let unlisten: (() => void) | undefined;
    let alive = true;
    void listen<{ kind: string; done: number; total: number }>(
      "tts-transform-progress",
      (event) => {
        if (event.payload.kind === "assist")
          setProgress({ done: event.payload.done, total: event.payload.total });
      },
    )
      .then((fn) => {
        if (alive) unlisten = fn;
        else fn();
      })
      .catch(() => {});
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [busy]);

  const assist = async (task: AssistTask) => {
    if (busy || readOnly || empty) return;
    const sel = selection();
    const input = buildAssistInput(value, sel.start, sel.end);
    if (input.sent.length === 0) return;
    const before = value;
    setBusy(task);
    setProgress(null);
    const result = await commands.ttsTextAssist(task, input.prompt);
    setBusy(null);
    setProgress(null);
    if (latest.current !== before) {
      toast(t("tts.editorMenu.changedMeanwhile"));
      return;
    }
    if (result.status === "error") {
      if (!/abgebrochen|cancel/i.test(result.error)) toast.error(result.error);
      return;
    }
    const applied =
      task === "grammar"
        ? applyGrammar(before, input, result.data)
        : applyStructure(before, input, result.data);
    if (applied.changes === 0 || applied.text === before) {
      toast(
        task === "grammar"
          ? t("tts.editorMenu.grammarNothing")
          : t("tts.editorMenu.structureNothing"),
      );
      return;
    }
    onChange(applied.text);
    toast(
      t(
        task === "grammar"
          ? "tts.editorMenu.grammarDone"
          : "tts.editorMenu.structureDone",
        { count: applied.changes },
      ) +
        (applied.rejected > 0
          ? ` ${t("tts.editorMenu.rejected", { count: applied.rejected })}`
          : ""),
      {
        action: {
          label: t("tts.tidyUndo"),
          onClick: () => {
            // Nur zurück, solange niemand weitergeschrieben hat.
            if (latest.current === applied.text) onChange(before);
            else toast(t("tts.editorMenu.changedMeanwhile"));
          },
        },
      },
    );
  };

  const cancelAssist = () => void commands.ttsTransformCancel("assist");

  // ---- Tastenkürzel -------------------------------------------------------
  const openFind = useCallback((withReplace: boolean) => {
    setFindOpen(true);
    setFindFocus((n) => n + 1);
    if (withReplace) setReplaceOpen(true);
  }, []);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (
        !(event.ctrlKey || event.metaKey) ||
        event.altKey ||
        event.shiftKey ||
        event.defaultPrevented
      )
        return;
      // Nur im Editor-Bereich, nicht in Dialogen oder anderen Feldern.
      const target = event.target as Element | null;
      if (target?.closest?.('[role="dialog"]')) return;
      if (
        target instanceof HTMLInputElement &&
        !target.closest('[data-testid="editor-menu"]')
      )
        return;
      const key = event.key.toLowerCase();
      if (key !== "f" && key !== "h") return;
      event.preventDefault();
      openFind(key === "h");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [openFind]);

  const busyLabel =
    busy && progress && progress.total > 1
      ? t("tts.editorMenu.busyProgress", {
          done: progress.done,
          total: progress.total,
        })
      : t("tts.editorMenu.busy");

  return (
    <div className="flex flex-col gap-1" data-testid="editor-menu">
      <div
        role="toolbar"
        aria-label={t("tts.editorMenu.label")}
        className="flex flex-wrap items-center gap-0.5 rounded-md border border-mid-gray/20 bg-mid-gray/5 px-1 py-0.5"
      >
        {labeled ? (
          <Button
            size="sm"
            variant="secondary"
            className={findOpen ? TOGGLE_ON : ""}
            icon={<Search width={16} height={16} aria-hidden="true" />}
            data-testid="editor-menu-find"
            aria-pressed={findOpen}
            title={t("tts.editorMenu.findHint")}
            onClick={() => {
              if (findOpen) {
                setFindOpen(false);
                setReplaceOpen(false);
              } else openFind(true);
            }}
          >
            {t("tts.editorMenu.find")}
          </Button>
        ) : (
          <IconAction
            icon={Search}
            size="sm"
            ghost
            label={t("tts.editorMenu.find")}
            description={t("tts.editorMenu.findHint")}
            testId="editor-menu-find"
            aria-pressed={findOpen}
            className={findOpen ? TOGGLE_ON : ""}
            onClick={() => {
              setFindOpen((v) => !v);
              if (findOpen) setReplaceOpen(false);
            }}
          />
        )}
        <IconAction
          icon={SpellCheck}
          size="sm"
          ghost
          label={t("tts.editorMenu.spellcheck")}
          description={t("tts.editorMenu.spellcheckHint")}
          testId="editor-menu-spellcheck"
          aria-pressed={spellCheck}
          className={spellCheck ? TOGGLE_ON : ""}
          onClick={() => onSpellCheck(!spellCheck)}
        />
        <span className={SEP} aria-hidden="true" />
        {FMT.map((f) => (
          <IconAction
            key={f.kind}
            icon={f.icon}
            size="sm"
            ghost
            label={t(`tts.editorMenu.format.${f.key}`)}
            description={t(`tts.editorMenu.format.${f.key}Hint`)}
            testId={`editor-menu-format-${f.kind}`}
            disabled={readOnly}
            onClick={() => format(f.kind)}
          />
        ))}
        <span className={SEP} aria-hidden="true" />
        <ActionMenu
          menuLabel={t("tts.editorMenu.cleanup")}
          widthClass="w-80"
          trigger={{
            icon: Eraser,
            size: "sm",
            ghost: true,
            label: t("tts.editorMenu.cleanup"),
            description: t("tts.editorMenu.cleanupHint"),
            testId: "editor-menu-cleanup",
            // Mit Schalter „Beim Einfügen bereinigen“ bleibt das Menü auch bei
            // leerem Text erreichbar; die Regeln sind dann einzeln gesperrt.
            disabled: readOnly || (empty && !onPasteCleanup),
          }}
          items={[
            {
              id: "all",
              label: t("tts.editorMenu.rules.all"),
              title: t("tts.editorMenu.rules.allHint"),
              icon: ListChecks,
              testId: "editor-cleanup-all",
              disabled: empty,
              onSelect: () => cleanup(CLEANUP_RULES),
            },
            ...CLEANUP_RULES.map((rule) => ({
              id: rule,
              label: t(`tts.editorMenu.rules.${rule}`),
              title: t(`tts.editorMenu.rules.${rule}Hint`),
              icon: RULE_ICONS[rule],
              testId: `editor-cleanup-${rule}`,
              disabled: empty,
              onSelect: () => cleanup([rule]),
            })),
            ...(onPasteCleanup
              ? [
                  {
                    id: "paste",
                    label: t("tts.editorMenu.pasteCleanup"),
                    title: t("tts.editorMenu.pasteCleanupHint"),
                    icon: ClipboardCheck,
                    testId: "editor-cleanup-paste",
                    checked: pasteCleanup,
                    dividerBefore: true,
                    onSelect: () => onPasteCleanup(!pasteCleanup),
                  },
                ]
              : []),
          ]}
        />
        <ActionMenu
          menuLabel={t("tts.editorMenu.ai")}
          widthClass="w-80"
          trigger={{
            icon: Sparkles,
            iconClassName: busy || tidying ? "animate-pulse" : undefined,
            size: "sm",
            ghost: true,
            label: busy ? busyLabel : t("tts.editorMenu.ai"),
            description: t("tts.editorMenu.aiHint"),
            testId: "editor-menu-ai",
            disabled: readOnly || empty,
          }}
          items={[
            {
              id: "grammar",
              label: t("tts.editorMenu.aiGrammar"),
              title: t("tts.editorMenu.aiGrammarHint"),
              icon: SpellCheck,
              testId: "editor-ai-grammar",
              disabled: busy !== null,
              onSelect: () => void assist("grammar"),
            },
            {
              id: "structure",
              label: t("tts.editorMenu.aiStructure"),
              title: t("tts.editorMenu.aiStructureHint"),
              icon: WrapText,
              testId: "editor-ai-structure",
              disabled: busy !== null,
              onSelect: () => void assist("structure"),
            },
            ...(onTidy
              ? [
                  {
                    id: "tidy",
                    label: t("tts.tidy"),
                    title: t("tts.tidyHint"),
                    icon: Wand2,
                    testId: "editor-ai-tidy",
                    disabled: tidying || busy !== null,
                    onSelect: onTidy,
                  },
                ]
              : []),
            ...(busy
              ? [
                  {
                    id: "cancel",
                    label: t("tts.editorMenu.aiCancel"),
                    icon: X,
                    testId: "editor-ai-cancel",
                    onSelect: cancelAssist,
                  },
                ]
              : []),
          ]}
        />
        {busy && (
          <span
            className="ms-1 truncate text-xs text-text/70"
            role="status"
            data-testid="editor-menu-busy"
          >
            {busyLabel}
          </span>
        )}
      </div>
      {findOpen && (
        <FindBar
          value={value}
          onChange={onChange}
          readOnly={readOnly}
          apiRef={apiRef}
          showReplace={replaceOpen}
          labeled={labeled}
          focusSignal={findFocus}
          onShowReplace={setReplaceOpen}
          onClose={() => {
            setFindOpen(false);
            setReplaceOpen(false);
          }}
        />
      )}
    </div>
  );
};

interface FindBarProps {
  value: string;
  onChange(next: string): void;
  readOnly: boolean;
  apiRef: React.RefObject<ChipEditorInsertApi | null>;
  showReplace: boolean;
  labeled: boolean;
  focusSignal: number;
  onShowReplace(on: boolean): void;
  onClose(): void;
}

const FIELD =
  "h-7 min-w-[9rem] flex-1 rounded border border-mid-gray/40 bg-background px-2 text-sm focus:border-logo-primary focus:outline-none";

const FindBar: React.FC<FindBarProps> = ({
  value,
  onChange,
  readOnly,
  apiRef,
  showReplace,
  labeled,
  focusSignal,
  onShowReplace,
  onClose,
}) => {
  const findId = useId();
  const replaceId = useId();
  const { t } = useTranslation();
  const [query, setQuery] = useState(() => {
    const sel = apiRef.current?.getSelection?.();
    const picked = sel ? value.slice(sel.start, sel.end) : "";
    return picked.includes("\n") ? "" : picked;
  });
  const [replacement, setReplacement] = useState("");
  const [opts, setOpts] = useState<FindOptions>({
    caseSensitive: false,
    wholeWord: false,
    regex: false,
  });
  const [current, setCurrent] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [focusSignal]);

  const hits = useMemo(() => findAll(value, query, opts), [value, query, opts]);
  const index = hits.length === 0 ? -1 : Math.min(current, hits.length - 1);

  const reveal = (i: number) => {
    const hit = hits[i];
    if (!hit) return;
    setCurrent(i);
    apiRef.current?.revealRange?.(hit.start, hit.end);
    // Fokus bleibt im Suchfeld, damit Enter weiterspringt.
    window.requestAnimationFrame(() => inputRef.current?.focus());
  };
  const step = (delta: number) => {
    if (hits.length === 0) return;
    reveal((index + delta + hits.length) % hits.length);
  };

  const replaceOne = () => {
    const hit = hits[index];
    if (!hit || readOnly) return;
    const insert = replacementAt(value, hit.start, query, replacement, opts);
    if (insert === null) return;
    const api = apiRef.current;
    if (api?.replaceRange) api.replaceRange(hit.start, hit.end, insert);
    else onChange(value.slice(0, hit.start) + insert + value.slice(hit.end));
  };
  const replaceEvery = () => {
    if (readOnly) return;
    const result = replaceAll(value, query, replacement, opts);
    if (result.changes === 0) return;
    onChange(result.text);
    toast(t("tts.editorMenu.replacedAll", { count: result.changes }));
  };

  const toggle = (key: keyof FindOptions) =>
    setOpts((o) => ({ ...o, [key]: !o[key] }));

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
    }
  };

  const optButton = (key: keyof FindOptions, icon: LucideIcon) => (
    <IconAction
      icon={icon}
      size="sm"
      ghost
      label={t(`tts.editorMenu.opt.${key}`)}
      description={t(`tts.editorMenu.opt.${key}Hint`)}
      testId={`find-opt-${key}`}
      aria-pressed={opts[key]}
      className={opts[key] ? TOGGLE_ON : ""}
      onClick={() => toggle(key)}
    />
  );

  return (
    <div
      className="flex flex-col gap-1 rounded-md border border-mid-gray/20 bg-mid-gray/5 p-1"
      role="search"
      aria-label={t("tts.editorMenu.find")}
      data-testid="find-bar"
      onKeyDown={onKeyDown}
    >
      <div className="flex flex-wrap items-center gap-1">
        {labeled && (
          <label htmlFor={findId} className="w-28 shrink-0 text-sm font-medium">
            {t("tts.editorMenu.findLabel")}
          </label>
        )}
        <input
          id={findId}
          ref={inputRef}
          className={FIELD}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setCurrent(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              step(e.shiftKey ? -1 : 1);
            }
          }}
          placeholder={t("tts.editorMenu.findPlaceholder")}
          aria-label={labeled ? undefined : t("tts.editorMenu.findPlaceholder")}
          data-testid="find-input"
        />
        <span
          className="min-w-16 text-center text-xs tabular-nums text-text/70"
          aria-live="polite"
          data-testid="find-count"
        >
          {query
            ? hits.length === 0
              ? t("tts.editorMenu.noHits")
              : t("tts.editorMenu.hitOf", { n: index + 1, total: hits.length })
            : ""}
        </span>
        <IconAction
          icon={ChevronUp}
          size="sm"
          ghost
          label={t("tts.editorMenu.prev")}
          description={t("tts.editorMenu.prevHint")}
          testId="find-prev"
          disabled={hits.length === 0}
          onClick={() => step(-1)}
        />
        <IconAction
          icon={ChevronDown}
          size="sm"
          ghost
          label={t("tts.editorMenu.next")}
          description={t("tts.editorMenu.nextHint")}
          testId="find-next"
          disabled={hits.length === 0}
          onClick={() => step(1)}
        />
        {optButton("caseSensitive", CaseSensitive)}
        {optButton("wholeWord", WholeWord)}
        {optButton("regex", Regex)}
        <IconAction
          icon={Replace}
          size="sm"
          ghost
          label={t("tts.editorMenu.toggleReplace")}
          description={t("tts.editorMenu.toggleReplaceHint")}
          testId="find-toggle-replace"
          aria-pressed={showReplace}
          className={showReplace ? TOGGLE_ON : ""}
          onClick={() => onShowReplace(!showReplace)}
        />
        <IconAction
          icon={X}
          size="sm"
          ghost
          label={t("tts.editorMenu.closeFind")}
          description={t("tts.editorMenu.closeFindHint")}
          testId="find-close"
          onClick={onClose}
        />
      </div>
      {showReplace && (
        <div className="flex flex-wrap items-center gap-1">
          {labeled && (
            <label
              htmlFor={replaceId}
              className="w-28 shrink-0 text-sm font-medium"
            >
              {t("tts.editorMenu.replaceLabel")}
            </label>
          )}
          <input
            id={replaceId}
            className={FIELD}
            value={replacement}
            onChange={(e) => setReplacement(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                if (e.ctrlKey || e.metaKey) replaceEvery();
                else replaceOne();
              }
            }}
            placeholder={t("tts.editorMenu.replacePlaceholder")}
            aria-label={
              labeled ? undefined : t("tts.editorMenu.replacePlaceholder")
            }
            data-testid="replace-input"
            disabled={readOnly}
          />
          {labeled ? (
            <>
              <Button
                size="sm"
                variant="secondary"
                icon={<Replace width={16} height={16} aria-hidden="true" />}
                data-testid="replace-one"
                title={t("tts.editorMenu.replaceOneHint")}
                disabled={readOnly || hits.length === 0}
                onClick={replaceOne}
              >
                {t("tts.editorMenu.replaceOne")}
              </Button>
              <Button
                size="sm"
                variant="primary"
                icon={<ReplaceAll width={16} height={16} aria-hidden="true" />}
                data-testid="replace-all"
                title={t("tts.editorMenu.replaceAllHint")}
                disabled={readOnly || hits.length === 0}
                onClick={replaceEvery}
              >
                {t("tts.editorMenu.replaceAll")}
              </Button>
            </>
          ) : (
            <>
              <IconAction
                icon={Replace}
                size="sm"
                ghost
                label={t("tts.editorMenu.replaceOne")}
                description={t("tts.editorMenu.replaceOneHint")}
                testId="replace-one"
                disabled={readOnly || hits.length === 0}
                onClick={replaceOne}
              />
              <IconAction
                icon={ReplaceAll}
                size="sm"
                ghost
                label={t("tts.editorMenu.replaceAll")}
                description={t("tts.editorMenu.replaceAllHint")}
                testId="replace-all"
                disabled={readOnly || hits.length === 0}
                onClick={replaceEvery}
              />
            </>
          )}
        </div>
      )}
    </div>
  );
};
