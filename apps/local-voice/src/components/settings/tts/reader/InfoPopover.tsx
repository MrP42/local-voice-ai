import React, { useEffect, useId, useState } from "react";
import { Info } from "lucide-react";

interface InfoPopoverProps {
  label: string;
  children: React.ReactNode;
  testId?: string;
}

/**
 * Ein ⓘ-Knopf mit aufklappendem Hinweis: Wissen, das man einmal liest
 * (Schreibregeln), steht nicht dauerhaft im Weg. Escape und Klick daneben
 * schließen; der Inhalt ist per `aria-controls` mit dem Knopf verbunden.
 */
export const InfoPopover: React.FC<InfoPopoverProps> = ({
  label,
  children,
  testId,
}) => {
  const [open, setOpen] = useState(false);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [open]);
  return (
    <div className="relative">
      <button
        type="button"
        className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        aria-expanded={open}
        aria-controls={id}
        aria-label={label}
        title={label}
        onClick={() => setOpen((o) => !o)}
        data-testid={testId}
      >
        <Info width={16} height={16} aria-hidden="true" />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-30" onClick={() => setOpen(false)} />
          <div
            id={id}
            role="dialog"
            aria-label={label}
            className="absolute right-0 top-full z-40 mt-1 w-80 max-w-[calc(100vw-2rem)] rounded-lg border border-mid-gray/40 bg-background p-3 text-sm text-text shadow-lg"
            data-testid={testId ? `${testId}-panel` : undefined}
          >
            {children}
          </div>
        </>
      )}
    </div>
  );
};
