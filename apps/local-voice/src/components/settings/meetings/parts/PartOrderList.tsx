import React from "react";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp } from "lucide-react";

export interface OrderItem {
  id: string;
  label: string;
  /** Zweite Zeile, z. B. Datum und Dauer. */
  detail?: string;
}

const MOVE_BTN =
  "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/20 hover:text-text disabled:cursor-not-allowed disabled:opacity-30 max-sm:h-[44px] max-sm:w-[44px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60";

/**
 * Geordnete Liste der Teile einer Besprechung mit "nach oben" / "nach unten".
 * Die Reihenfolge ist die Zeitachse: Teil 1 beginnt bei 0:00, Teil 2 direkt
 * nach dem Ende von Teil 1. Pfeile statt Ziehen: per Tastatur und Touch
 * bedienbar, Touchziele unter 640 px mindestens 44 x 44 px.
 */
export const PartOrderList: React.FC<{
  items: OrderItem[];
  onChange: (items: OrderItem[]) => void;
  testId: string;
}> = ({ items, onChange, testId }) => {
  const { t } = useTranslation();
  const move = (from: number, to: number) => {
    if (to < 0 || to >= items.length) return;
    const next = items.slice();
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    onChange(next);
  };
  return (
    <ol
      className="divide-y divide-mid-gray/20 rounded-md border border-mid-gray/30"
      data-testid={testId}
      aria-label={t("meetings.parts.orderLabel")}
    >
      {items.map((item, index) => (
        <li
          key={item.id}
          className="flex items-center gap-2 px-2 py-1"
          data-testid={`${testId}-item`}
        >
          <span
            className="w-6 shrink-0 text-center text-xs font-semibold tabular-nums text-text/60"
            aria-hidden="true"
          >
            {index + 1}
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-sm" title={item.label}>
              {item.label}
            </span>
            {item.detail && (
              <span className="block truncate text-xs text-text/60">
                {item.detail}
              </span>
            )}
          </span>
          <button
            type="button"
            className={MOVE_BTN}
            disabled={index === 0}
            onClick={() => move(index, index - 1)}
            aria-label={t("meetings.parts.moveUp", { name: item.label })}
            title={t("meetings.parts.moveUpShort")}
            data-testid={`${testId}-up`}
          >
            <ArrowUp width={16} height={16} aria-hidden="true" />
          </button>
          <button
            type="button"
            className={MOVE_BTN}
            disabled={index === items.length - 1}
            onClick={() => move(index, index + 1)}
            aria-label={t("meetings.parts.moveDown", { name: item.label })}
            title={t("meetings.parts.moveDownShort")}
            data-testid={`${testId}-down`}
          >
            <ArrowDown width={16} height={16} aria-hidden="true" />
          </button>
        </li>
      ))}
    </ol>
  );
};
