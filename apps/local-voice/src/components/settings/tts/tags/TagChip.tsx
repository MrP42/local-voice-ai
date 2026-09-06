import React from "react";
import type { LucideIcon } from "lucide-react";

export type TagChipState = "normal" | "active" | "suggestion" | "unverified";

interface TagChipProps {
  label: string;
  state?: TagChipState;
  icon?: LucideIcon;
  onClick?: () => void;
  onPointerDown?: (event: React.PointerEvent<HTMLButtonElement>) => void;
  title?: string;
  className?: string;
}

/**
 * Reines Render-Element für ein Tag — keine eigene Logik, kein State. Die
 * TagPalette benutzt es fürs Chip-Grid; ein späteres Paket (Editor-Mirror)
 * zeigt damit dieselben Chips für Tags, die schon im Text stehen.
 *
 * "unverified" ist bernstein statt gelb: das Tag steht nicht in der offiziellen
 * Tag-Liste von Fish Audio. Kein Fehler — das Modell nimmt Freitext entgegen —
 * aber auch keine Zusicherung, dass es ihm folgt statt ihn vorzulesen. Die
 * Farbe unterscheidet die beiden Klassen, ohne eine davon zu verbieten.
 *
 * Gelb ist die einzige Akzentfarbe des Design-Systems, und Gelb auf Hell hat
 * zu wenig Kontrast für Schrift (~1,1:1) — deshalb bleibt die Textfarbe in
 * "normal" und "suggestion" die normale Tinte (`text-text`), nur Hintergrund
 * und Rand werden gelb angedeutet. Einzige Ausnahme ist "active": dort füllt
 * Gelb den ganzen Chip, und die Schrift wechselt auf `--color-on-accent`
 * (Ink) — nie Weiß, das Kontrastverhältnis wäre zu gering.
 *
 * Der äußere Button traegt bewusst KEIN eigenes Innenpolster — wer ihn in
 * einer Palette mit 44px-Mindest-Klickflaeche braucht, gibt das per
 * `className` (z. B. `p-2`) dazu; ein Editor-Mirror mitten im Fließtext will
 * genau das nicht.
 */
export const TagChip: React.FC<TagChipProps> = ({
  label,
  state = "normal",
  icon: Icon,
  onClick,
  onPointerDown,
  title,
  className = "",
}) => {
  const pillClasses =
    state === "active"
      ? "bg-logo-primary text-on-accent border-logo-primary"
      : state === "unverified"
        ? "bg-amber-500/15 dark:bg-amber-500/20 text-text border-amber-500/50"
        : state === "suggestion"
          ? "bg-logo-primary/15 dark:bg-logo-primary/20 text-text border-logo-primary/40 border-dashed"
          : "bg-logo-primary/15 dark:bg-logo-primary/20 text-text border-logo-primary/40";

  return (
    <button
      type="button"
      onClick={onClick}
      onPointerDown={onPointerDown}
      title={title}
      className={`inline-flex items-center justify-center rounded cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${className}`}
    >
      <span
        className={`inline-flex max-w-full items-center gap-1 rounded border px-2 py-1 text-xs font-medium leading-none transition-colors ${pillClasses}`}
      >
        {Icon && <Icon width={12} height={12} aria-hidden="true" />}
        <span className="truncate">{label}</span>
      </span>
    </button>
  );
};
