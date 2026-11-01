import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  AudioLines,
  Mic,
  MicOff,
  Plus,
  SendHorizontal,
  Square,
  X,
} from "lucide-react";

/**
 * Eingabezeile eines Gesprächs: Anhang-Chips, Textfeld, darunter ＋-Menü,
 * Statusfläche, Stumm- und Senden-Knopf.
 *
 * Rein darstellend (ADR-0021): kein Store, kein Backend, keine Übersetzung.
 * Alles kommt über Props; die Komponente zieht später als Baustein in ein
 * gemeinsames UI-Paket um.
 */
export interface ComposerMenuItem {
  id: string;
  label: string;
  icon?: React.ReactNode;
  /** Erklärung unter der Beschriftung. */
  hint?: string;
  disabled?: boolean;
  /** Trennlinie vor dem Eintrag. */
  separatorBefore?: boolean;
  onSelect?: () => void;
}

export interface ComposerAttachment {
  id: string;
  label: string;
  /** Zusatz neben dem Namen, z. B. „gekürzt“. */
  detail?: string;
}

export interface ComposerLabels {
  message: string;
  placeholder: string;
  add: string;
  mute: string;
  send: string;
  stop: string;
  removeAttachment: (label: string) => string;
}

/** Runder Knopf für das Live-Sprachgespräch rechts neben dem Mikrofon. */
export interface ComposerCallAction {
  /** Ein Gespräch läuft: der Knopf beendet es (rot). */
  active: boolean;
  /** Beschriftung (Name und Tooltip) je nach Zustand. */
  label: string;
  onClick: () => void;
}

export interface ComposerProps {
  value: string;
  onChange: (value: string) => void;
  onSend: () => void;
  labels: ComposerLabels;
  attachments?: ComposerAttachment[];
  onRemoveAttachment?: (id: string) => void;
  menuItems?: ComposerMenuItem[];
  /** Statusanzeige (Welle, Zustand) zwischen Menü und Knöpfen. */
  children?: React.ReactNode;
  /** Links neben dem ＋-Menü (Berechtigungs-Chip). */
  leading?: React.ReactNode;
  /** Rechts vor dem Mikrofon (Modellauswahl). */
  tools?: React.ReactNode;
  /** Live-Gespräch starten bzw. beenden. */
  callAction?: ComposerCallAction;
  muted?: boolean;
  onToggleMute?: () => void;
  /** Senden läuft: Senden-Knopf gesperrt. */
  sending?: boolean;
  /** Gesetzt: während `sending` ersetzt ein Stopp-Knopf den Senden-Knopf. */
  onStop?: () => void;
  /** Eingabe gesperrt (ein anderes Gespräch wird gerade geladen). */
  disabled?: boolean;
}

export const Composer: React.FC<ComposerProps> = ({
  value,
  onChange,
  onSend,
  labels,
  attachments = [],
  onRemoveAttachment,
  menuItems = [],
  children,
  leading,
  tools,
  callAction,
  muted = false,
  onToggleMute,
  sending = false,
  onStop,
  disabled = false,
}) => {
  const [menuOpen, setMenuOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const menuId = useId();
  const canSend =
    (value.trim().length > 0 || attachments.length > 0) &&
    !sending &&
    !disabled;

  useLayoutEffect(() => {
    const el = areaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [value]);

  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  const roundBtn =
    "grid place-items-center w-9 h-9 shrink-0 rounded-full cursor-pointer transition-colors hover:bg-mid-gray/15 disabled:cursor-not-allowed disabled:opacity-50";

  return (
    <div className="flex justify-center px-4 pb-3 pt-2">
      <div
        ref={rootRef}
        className="relative w-full max-w-[880px] rounded-2xl border border-mid-gray/30 bg-background shadow-lg p-2 focus-within:border-logo-primary"
      >
        {attachments.length > 0 && (
          <ul className="flex flex-wrap gap-1.5 px-1 pb-1.5 m-0 list-none">
            {attachments.map((a) => (
              <li
                key={a.id}
                className="inline-flex items-center gap-1.5 h-7 ps-2.5 pe-1 rounded-md border border-mid-gray/25 bg-mid-gray/10 text-xs"
              >
                <span className="truncate max-w-48">{a.label}</span>
                {a.detail && <small className="text-text/50">{a.detail}</small>}
                <button
                  type="button"
                  className="grid place-items-center w-5 h-5 rounded cursor-pointer hover:bg-mid-gray/20"
                  aria-label={labels.removeAttachment(a.label)}
                  onClick={() => onRemoveAttachment?.(a.id)}
                >
                  <X className="w-3 h-3" aria-hidden="true" />
                </button>
              </li>
            ))}
          </ul>
        )}
        <textarea
          ref={areaRef}
          rows={1}
          disabled={disabled}
          value={value}
          aria-label={labels.message}
          placeholder={labels.placeholder}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing)
              return;
            e.preventDefault();
            if (canSend) onSend();
          }}
          className="block w-full resize-none border-0 bg-transparent px-2 py-1.5 text-sm text-text outline-none placeholder:text-text/40 min-h-9"
        />
        <div
          className="flex items-center gap-1"
          role="toolbar"
          aria-label={labels.message}
        >
          {leading && <div className="min-w-0 shrink">{leading}</div>}
          {menuItems.length > 0 && (
            <div className="relative shrink-0">
              <button
                type="button"
                className={roundBtn}
                aria-label={labels.add}
                title={labels.add}
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                aria-controls={menuOpen ? menuId : undefined}
                onClick={() => setMenuOpen((o) => !o)}
              >
                <Plus className="w-[18px] h-[18px]" aria-hidden="true" />
              </button>
              {menuOpen && (
                <div
                  id={menuId}
                  role="menu"
                  aria-label={labels.add}
                  className="absolute bottom-full mb-2 start-0 w-72 rounded-lg border border-mid-gray/40 bg-background shadow-lg p-1.5 z-50 text-[13px]"
                >
                  {menuItems.map((item) => (
                    <React.Fragment key={item.id}>
                      {item.separatorBefore && (
                        <div className="my-1.5 h-px bg-mid-gray/20" />
                      )}
                      <button
                        type="button"
                        role="menuitem"
                        aria-disabled={item.disabled || undefined}
                        title={item.hint}
                        onClick={() => {
                          if (item.disabled) return;
                          setMenuOpen(false);
                          item.onSelect?.();
                        }}
                        className={`flex w-full items-start gap-2 rounded px-2 py-1.5 text-start ${
                          item.disabled
                            ? "opacity-60 cursor-not-allowed"
                            : "cursor-pointer hover:bg-mid-gray/15"
                        }`}
                      >
                        {item.icon && (
                          <span className="mt-0.5 shrink-0">{item.icon}</span>
                        )}
                        <span className="flex flex-col min-w-0">
                          <span>{item.label}</span>
                          {item.hint && (
                            <span className="text-xs text-text/55">
                              {item.hint}
                            </span>
                          )}
                        </span>
                      </button>
                    </React.Fragment>
                  ))}
                </div>
              )}
            </div>
          )}
          <span className="flex-1" />
          {children}
          {tools && <div className="min-w-0 shrink">{tools}</div>}
          {onToggleMute && (
            <button
              type="button"
              className={`${roundBtn} ${muted ? "bg-logo-primary/15 text-logo-primary" : ""}`}
              aria-label={labels.mute}
              title={labels.mute}
              aria-pressed={muted}
              onClick={onToggleMute}
            >
              {muted ? (
                <MicOff className="w-[18px] h-[18px]" aria-hidden="true" />
              ) : (
                <Mic className="w-[18px] h-[18px]" aria-hidden="true" />
              )}
            </button>
          )}
          {callAction && (
            <button
              type="button"
              data-testid="call-button"
              className={`grid place-items-center w-9 h-9 shrink-0 rounded-full cursor-pointer transition-colors ${
                callAction.active
                  ? "bg-red-600 text-white hover:bg-red-700"
                  : "bg-text/90 text-background hover:opacity-80"
              }`}
              aria-label={callAction.label}
              title={callAction.label}
              onClick={callAction.onClick}
            >
              <AudioLines className="w-[18px] h-[18px]" aria-hidden="true" />
            </button>
          )}
          {sending && onStop && (
            <button
              type="button"
              className="grid place-items-center w-9 h-9 shrink-0 rounded-full bg-logo-primary text-white cursor-pointer hover:opacity-90"
              aria-label={labels.stop}
              title={labels.stop}
              onClick={onStop}
            >
              <Square
                className="w-4 h-4"
                fill="currentColor"
                aria-hidden="true"
              />
            </button>
          )}
          {canSend && (
            <button
              type="button"
              className="grid place-items-center w-9 h-9 shrink-0 rounded-full bg-logo-primary text-white cursor-pointer hover:opacity-90"
              aria-label={labels.send}
              title={labels.send}
              onClick={onSend}
            >
              <SendHorizontal
                className="w-[18px] h-[18px]"
                aria-hidden="true"
              />
            </button>
          )}
        </div>
      </div>
    </div>
  );
};
