import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  Folder as FolderIcon,
  FolderMinus,
  FolderOpen,
  Folders,
} from "lucide-react";
import { DROP_ATTR } from "./useMeetingDrag";

export type ProjectKind = "all" | "project" | "none";

interface ProjectRowProps {
  /** Ziel-ID fuer das Ziehen: Projekt-ID, "all" oder "none". */
  id: string;
  kind: ProjectKind;
  name: string;
  count: number | null;
  selected: boolean;
  /** Aufgeklappt (nur sinnvoll, wenn gewaehlt): die Besprechungen stehen darunter. */
  open: boolean;
  /** Eine Besprechung wird gerade ueber diese Zeile gezogen. */
  dropActive: boolean;
  /** Nimmt abgelegte Besprechungen an (nicht "Alle Aufnahmen"). */
  dropTarget: boolean;
  onActivate: () => void;
  /** Umbenennen erlaubt (nur echte Projekte). */
  onContextMenu?: (x: number, y: number) => void;
  onRenameStart?: () => void;
  onMove?: (direction: -1 | 1) => void;
  /** Eingabe statt Name; liefert `null` bei Erfolg, sonst den Fehlertext. */
  editing?: boolean;
  onCommit?: (name: string) => Promise<string | null>;
  onCancel?: () => void;
}

const ICONS = {
  all: Folders,
  none: FolderMinus,
} as const;

/**
 * Eine Zeile der Projekte-Spalte. Klick waehlt (und klappt auf bzw. zu),
 * Doppelklick und F2 benennen um, Alt+Pfeil hoch/runter sortiert, die
 * Kontexttaste oeffnet das Menue. Beim Umbenennen und Anlegen ersetzt eine
 * Eingabe den Namen: Enter speichert, Escape verwirft, ein Klick daneben
 * speichert (leer = verwerfen).
 */
export const ProjectRow: React.FC<ProjectRowProps> = ({
  id,
  kind,
  name,
  count,
  selected,
  open,
  dropActive,
  dropTarget,
  onActivate,
  onContextMenu,
  onRenameStart,
  onMove,
  editing = false,
  onCommit,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(name);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  // Enter/Escape beenden die Eingabe selbst; das folgende blur darf nicht
  // ein zweites Mal speichern.
  const settled = useRef(false);

  useEffect(() => {
    if (!editing) return;
    settled.current = false;
    setDraft(name);
    setError(null);
    const input = inputRef.current;
    input?.focus();
    input?.select();
    // Nur beim Beginn der Eingabe: spaetere Namensaenderungen stoeren nicht.
  }, [editing]);

  const commit = async () => {
    if (settled.current) return;
    const value = draft.trim();
    if (value === "" || value === name) {
      settled.current = true;
      onCancel?.();
      return;
    }
    const failed = await onCommit?.(value);
    if (failed) {
      // Die Eingabe bleibt offen: Namen korrigieren, statt alles zu verlieren.
      setError(failed);
      inputRef.current?.focus();
      return;
    }
    settled.current = true;
  };

  const cancel = () => {
    settled.current = true;
    onCancel?.();
  };

  const Icon =
    kind === "project"
      ? open && selected
        ? FolderOpen
        : FolderIcon
      : ICONS[kind];
  const Chevron = open && selected ? ChevronDown : ChevronRight;

  if (editing) {
    return (
      <div className="px-1" data-testid="project-edit-row">
        <div className="flex min-h-[32px] items-center gap-1.5 rounded-lg border border-logo-primary bg-background px-2">
          <FolderIcon
            width={16}
            height={16}
            aria-hidden="true"
            className="shrink-0 text-text/50"
          />
          <input
            ref={inputRef}
            value={draft}
            maxLength={60}
            aria-label={t("meetings.projects.nameLabel")}
            aria-invalid={error !== null}
            placeholder={t("meetings.projects.namePlaceholder")}
            className="h-7 min-w-0 flex-1 bg-transparent text-sm outline-none"
            onChange={(e) => {
              setDraft(e.target.value);
              setError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void commit();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                cancel();
              }
            }}
            onBlur={() => void commit()}
          />
        </div>
        {error && (
          <p role="alert" className="px-2 pt-0.5 text-xs text-red-400">
            {error}
          </p>
        )}
      </div>
    );
  }

  return (
    <button
      type="button"
      data-testid="project-row"
      data-project-id={id}
      {...(dropTarget ? { [DROP_ATTR]: id } : {})}
      aria-current={selected ? "true" : undefined}
      aria-expanded={selected ? open : undefined}
      className={`flex min-h-[32px] w-full cursor-pointer items-center gap-1.5 rounded-lg border px-2 text-start text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 ${
        dropActive
          ? "border-logo-primary bg-logo-primary/15"
          : selected
            ? "border-transparent bg-logo-primary/15 font-semibold"
            : "border-transparent hover:bg-mid-gray/10"
      }`}
      // Der zweite Klick eines Doppelklicks waehlt nicht noch einmal (sonst
      // klappte die Zeile genau beim Umbenennen wieder zu).
      onClick={(e) => {
        if (e.detail < 2) onActivate();
      }}
      onDoubleClick={onRenameStart}
      onContextMenu={
        onContextMenu
          ? (e) => {
              e.preventDefault();
              onContextMenu(e.clientX, e.clientY);
            }
          : undefined
      }
      onKeyDown={(e) => {
        if (e.key === "F2" && onRenameStart) {
          e.preventDefault();
          onRenameStart();
        } else if (e.altKey && onMove && /^Arrow(Up|Down)$/.test(e.key)) {
          e.preventDefault();
          onMove(e.key === "ArrowUp" ? -1 : 1);
          // Der Fokus bleibt an der Zeile, auch wenn React sie umhaengt.
          window.setTimeout(() => {
            document
              .querySelector<HTMLElement>(`[data-project-id="${id}"]`)
              ?.focus();
          }, 0);
        }
      }}
    >
      <Chevron
        width={14}
        height={14}
        aria-hidden="true"
        className="shrink-0 text-text/40"
      />
      <Icon
        width={16}
        height={16}
        aria-hidden="true"
        className="shrink-0 text-text/60"
      />
      <span className="min-w-0 flex-1 truncate">{name}</span>
      {count !== null && (
        <span
          className="shrink-0 text-xs font-medium tabular-nums text-text/50"
          data-testid="project-count"
        >
          {count}
        </span>
      )}
    </button>
  );
};
