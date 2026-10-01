import React, { useCallback, useLayoutEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import type { NoteBlock } from "@/bindings";
import {
  blockFromMarkdownPrefix,
  formatAt,
  makeBlock,
  mergeWithPrevious,
  splitBlock,
} from "@/lib/meetingNotes";

/**
 * Block-Editor fuer Stichpunkte (M1, P1c): eine Liste auto-wachsender
 * Textareas, kein Rich-Text-Editor und keine neue Abhaengigkeit.
 *
 * Tastatur: Enter = neuer Block (Shift+Enter = Zeilenumbruch im Block),
 * Backspace am Blockanfang = mit dem vorigen verschmelzen, Kuerzel am
 * Absatzanfang: `# ` Ueberschrift, `- ` Stichpunkt, `[ ] ` Aufgabe.
 * Jeder Block traegt links grau die Audioposition seiner Anlage (`mm:ss`).
 */

type Updater = (prev: NoteBlock[]) => NoteBlock[];

interface NoteBlocksEditorProps {
  blocks: NoteBlock[];
  onChange: (updater: Updater) => void;
  /** Audioposition fuer einen neu angelegten Block (nur waehrend der Aufnahme). */
  stampNewBlock?: () => Promise<number | null>;
  showTimestamps?: boolean;
  placeholder: string;
  /** Der Fokus hat den Editor verlassen (Autosave sofort ausloesen). */
  onBlur?: () => void;
  autoFocus?: boolean;
}

interface BlockRowProps {
  block: NoteBlock;
  showTimestamps: boolean;
  placeholder?: string;
  register: (id: string, el: HTMLTextAreaElement | null) => void;
  onText: (id: string, value: string, caret: number) => void;
  onKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>, id: string) => void;
  onToggle: (id: string, checked: boolean) => void;
}

const BlockRow: React.FC<BlockRowProps> = ({
  block,
  showTimestamps,
  placeholder,
  register,
  onText,
  onKeyDown,
  onToggle,
}) => {
  const { t } = useTranslation();
  const ref = useRef<HTMLTextAreaElement | null>(null);

  // Hoehe folgt dem Inhalt (kein eigener Scrollbalken je Block).
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [block.text, block.kind]);

  const textClass =
    block.kind === "heading"
      ? "text-base font-semibold"
      : block.kind === "todo" && block.checked
        ? "text-sm text-text/50 line-through"
        : "text-sm";

  return (
    <div
      data-block-id={block.id}
      data-kind={block.kind}
      className="flex items-start gap-2"
    >
      {showTimestamps && (
        <span
          data-testid="note-time"
          className="w-11 shrink-0 text-xs leading-6 tabular-nums text-text/40 select-none"
        >
          {block.at_ms !== null ? formatAt(block.at_ms) : ""}
        </span>
      )}
      {block.kind === "bullet" && (
        <span
          aria-hidden="true"
          className="w-3 shrink-0 text-center text-sm leading-6 text-text/50 select-none"
        >
          •
        </span>
      )}
      {block.kind === "todo" && (
        <input
          type="checkbox"
          checked={block.checked}
          onChange={(e) => onToggle(block.id, e.target.checked)}
          aria-label={t("meetings.notes.todoCheck")}
          className="mt-1.5 shrink-0 accent-logo-primary"
        />
      )}
      <textarea
        ref={(el) => {
          ref.current = el;
          register(block.id, el);
        }}
        rows={1}
        value={block.text}
        placeholder={placeholder}
        aria-label={t("meetings.notes.blockLabel")}
        onChange={(e) =>
          onText(block.id, e.target.value, e.target.selectionStart)
        }
        onKeyDown={(e) => onKeyDown(e, block.id)}
        className={`min-w-0 flex-1 resize-none overflow-hidden bg-transparent py-0 leading-6 text-text placeholder:text-text/40 focus:outline-none ${textClass}`}
      />
    </div>
  );
};

export const NoteBlocksEditor: React.FC<NoteBlocksEditorProps> = ({
  blocks,
  onChange,
  stampNewBlock,
  showTimestamps = true,
  placeholder,
  onBlur,
  autoFocus = false,
}) => {
  const fields = useRef(new Map<string, HTMLTextAreaElement>());
  const pendingFocus = useRef<{ id: string; pos: number } | null>(null);
  const blocksRef = useRef(blocks);
  blocksRef.current = blocks;

  // Fokus nach dem Rendern setzen (Block wurde gerade angelegt/verschmolzen).
  useLayoutEffect(() => {
    const pending = pendingFocus.current;
    if (!pending) return;
    const el = fields.current.get(pending.id);
    if (!el) return;
    pendingFocus.current = null;
    el.focus();
    el.setSelectionRange(pending.pos, pending.pos);
  });

  const register = useCallback((id: string, el: HTMLTextAreaElement | null) => {
    if (el) fields.current.set(id, el);
    else fields.current.delete(id);
  }, []);

  /** Neuer Block: Audioposition nachtragen, sobald das Backend sie liefert. */
  const stamp = useCallback(
    (id: string) => {
      if (!stampNewBlock) return;
      void stampNewBlock().then((ms) => {
        if (ms === null) return;
        onChange((prev) =>
          prev.map((b) =>
            b.id === id && b.at_ms === null ? { ...b, at_ms: ms } : b,
          ),
        );
      });
    },
    [stampNewBlock, onChange],
  );

  const focusAt = (id: string, pos: number) => {
    pendingFocus.current = { id, pos };
  };

  const handleText = (id: string, value: string, caret: number) => {
    onChange((prev) =>
      prev.map((b) => {
        if (b.id !== id) return b;
        if (b.kind === "paragraph" || b.kind === "bullet") {
          const converted = blockFromMarkdownPrefix(value);
          // Im Stichpunkt gilt nur `[ ] ` (wie `- [ ] ` in Markdown), nicht `# `.
          if (
            converted &&
            (b.kind === "paragraph" || converted.kind === "todo")
          ) {
            focusAt(
              id,
              Math.max(0, caret - (value.length - converted.text.length)),
            );
            return { ...b, ...converted };
          }
        }
        return { ...b, text: value };
      }),
    );
  };

  const handleStarter = (value: string, caret: number) => {
    if (value === "") return;
    const converted = blockFromMarkdownPrefix(value);
    const block = makeBlock(
      converted ?? { kind: "paragraph", text: value, checked: false },
    );
    focusAt(
      block.id,
      converted
        ? Math.max(0, caret - (value.length - converted.text.length))
        : caret,
    );
    onChange(() => [block]);
    stamp(block.id);
  };

  const handleKeyDown = (
    e: React.KeyboardEvent<HTMLTextAreaElement>,
    id: string,
  ) => {
    // Wahrend einer IME-Eingabe gehoert Enter dem Eingabemodus, nicht dem Editor.
    if (e.nativeEvent.isComposing) return;
    const el = e.currentTarget;
    const index = blocksRef.current.findIndex((b) => b.id === id);
    if (index < 0) return;

    if (e.key === "Enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      const fresh = makeBlock();
      const result = splitBlock(
        blocksRef.current,
        index,
        el.selectionStart,
        fresh,
        el.selectionEnd,
      );
      focusAt(result.focusId, 0);
      onChange(() => result.blocks);
      if (result.focusId === fresh.id) stamp(fresh.id);
      return;
    }

    if (
      e.key === "Backspace" &&
      el.selectionStart === 0 &&
      el.selectionEnd === 0
    ) {
      const merged = mergeWithPrevious(blocksRef.current, index);
      if (!merged) return;
      e.preventDefault();
      focusAt(merged.focusId, merged.caret);
      onChange(() => merged.blocks);
      return;
    }

    if (e.key === "ArrowUp" && el.selectionStart === 0 && index > 0) {
      const previous = blocksRef.current[index - 1];
      e.preventDefault();
      const target = fields.current.get(previous.id);
      target?.focus();
      target?.setSelectionRange(previous.text.length, previous.text.length);
      return;
    }
    if (
      e.key === "ArrowDown" &&
      el.selectionEnd === el.value.length &&
      index < blocksRef.current.length - 1
    ) {
      const next = blocksRef.current[index + 1];
      e.preventDefault();
      const target = fields.current.get(next.id);
      target?.focus();
      target?.setSelectionRange(0, 0);
    }
  };

  const handleToggle = (id: string, checked: boolean) =>
    onChange((prev) => prev.map((b) => (b.id === id ? { ...b, checked } : b)));

  return (
    <div
      data-testid="note-blocks"
      className="space-y-1"
      onBlur={(e) => {
        // Nur wenn der Fokus den ganzen Editor verlaesst, nicht beim Wechsel zwischen Bloecken.
        if (!e.currentTarget.contains(e.relatedTarget as Node | null))
          onBlur?.();
      }}
    >
      {blocks.length === 0 ? (
        <div className="flex items-start gap-2">
          {showTimestamps && <span className="w-11 shrink-0" />}
          <textarea
            data-testid="note-starter"
            rows={1}
            value=""
            autoFocus={autoFocus}
            placeholder={placeholder}
            aria-label={placeholder}
            onChange={(e) =>
              handleStarter(e.target.value, e.target.selectionStart)
            }
            className="min-w-0 flex-1 resize-none overflow-hidden bg-transparent py-0 text-sm leading-6 text-text placeholder:text-text/40 focus:outline-none"
          />
        </div>
      ) : (
        blocks.map((block, i) => (
          <BlockRow
            key={block.id}
            block={block}
            showTimestamps={showTimestamps}
            placeholder={
              blocks.length === 1 && i === 0 ? placeholder : undefined
            }
            register={register}
            onText={handleText}
            onKeyDown={handleKeyDown}
            onToggle={handleToggle}
          />
        ))
      )}
    </div>
  );
};
