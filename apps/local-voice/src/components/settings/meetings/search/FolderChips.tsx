import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { TFunction } from "i18next";

/** Fehlercodes der Ordner-Commands in Nutzertext. */
export const folderErrorText = (code: string, t: TFunction) => {
  switch (code) {
    case "folder_name_invalid":
      return t("meetings.folders.errors.nameInvalid");
    case "folder_name_taken":
      return t("meetings.folders.errors.nameTaken");
    case "folder_not_found":
      return t("meetings.folders.errors.notFound");
    default:
      return t("meetings.folders.errors.failed");
  }
};

// ---------------------------------------------------------------------------
// Kontextmenue (Projekte und Zeilen der Besprechungsliste)
// ---------------------------------------------------------------------------

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

interface ContextMenuProps {
  x: number;
  y: number;
  label: string;
  items: ContextMenuItem[];
  onClose: () => void;
}

/** Schlichtes Menue an der Mausposition; schliesst bei Klick daneben, Escape,
 *  Scrollen und Fensterwechsel. */
export const ContextMenu: React.FC<ContextMenuProps> = ({
  x,
  y,
  label,
  items,
  onClose,
}) => {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - rect.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - rect.height - 4)),
    });
    el.querySelector<HTMLElement>("[role=menuitem]")?.focus();
  }, [x, y]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onClose, true);
    window.addEventListener("blur", onClose);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onClose, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={label}
      className="fixed z-50 min-w-[180px] rounded-md border border-mid-gray/30 bg-background py-1 shadow-lg"
      style={{ left: pos.x, top: pos.y }}
    >
      {items.map((item) => (
        <button
          key={item.label}
          type="button"
          role="menuitem"
          className={`flex w-full min-h-[36px] cursor-pointer items-center px-3 text-start text-sm hover:bg-mid-gray/15 focus-visible:outline-none focus-visible:bg-mid-gray/15 ${
            item.danger ? "text-red-400" : "text-text/80 hover:text-text"
          }`}
          onClick={() => {
            onClose();
            item.onSelect();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>,
    document.body,
  );
};
