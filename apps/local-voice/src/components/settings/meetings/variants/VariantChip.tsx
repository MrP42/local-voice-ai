import React, { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown } from "lucide-react";
import type { TranscriptVariant } from "@/bindings";
import { ContextMenu, type ContextMenuItem } from "../search/FolderChips";
import { variantKindLabel, variantListLabel } from "./useVariants";

interface VariantChipProps {
  variants: TranscriptVariant[];
  /** „Fassung waehlen“: die Fassung wird das aktive Transkript. */
  onActivate: (variant: TranscriptVariant) => void;
  /** Oeffnet den Reiter „Vergleich“. */
  onCompare?: () => void;
  /** Weitere Eintraege unter der Liste (z. B. Untertitel laden). */
  extraItems?: ContextMenuItem[];
  disabled?: boolean;
}

/**
 * Der Fassungs-Chip im Kopf des Transkript-Reiters: zeigt die aktive Fassung
 * („Fassung v2 · Eigene Transkription“) und oeffnet die Liste aller Fassungen.
 */
export const VariantChip: React.FC<VariantChipProps> = ({
  variants,
  onActivate,
  onCompare,
  extraItems = [],
  disabled = false,
}) => {
  const { t } = useTranslation();
  const button = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const active = variants.find((v) => v.active) ?? variants[0];
  if (!active) return null;

  const open = () => {
    const rect = button.current?.getBoundingClientRect();
    setMenu({ x: rect?.left ?? 0, y: (rect?.bottom ?? 0) + 4 });
  };

  const items: ContextMenuItem[] = [
    ...variants.map((v) => {
      const label = variantListLabel(v, t);
      return {
        label: v.active ? t("meetings.variants.itemActive", { label }) : label,
        disabled: v.active,
        onSelect: () => onActivate(v),
      };
    }),
    ...(onCompare && variants.length > 1
      ? [{ label: t("meetings.variants.compare"), onSelect: onCompare }]
      : []),
    ...extraItems,
  ];

  return (
    <>
      <button
        ref={button}
        type="button"
        data-testid="variant-chip"
        aria-haspopup="menu"
        aria-expanded={menu !== null}
        aria-label={t("meetings.variants.chipLabel")}
        disabled={disabled}
        onClick={open}
        className="inline-flex max-w-full items-center gap-1 rounded-full border border-mid-gray/30 px-2.5 py-0.5 text-xs text-text/80 hover:bg-mid-gray/15 disabled:opacity-50 cursor-pointer"
      >
        <span className="truncate">
          {t("meetings.variants.chip", {
            number: active.number,
            kind: variantKindLabel(active, t),
          })}
        </span>
        <ChevronDown width={12} height={12} aria-hidden="true" />
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          label={t("meetings.variants.menuLabel")}
          items={items}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  );
};
