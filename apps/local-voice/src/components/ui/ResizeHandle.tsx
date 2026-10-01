import React, { useRef, useState } from "react";

/** Schrittweite der Pfeiltasten in Pixeln. */
export const RESIZE_KEY_STEP = 16;

const clamp = (value: number, min: number, max: number) =>
  Math.min(Math.max(value, min), Math.max(min, max));

/**
 * Senkrechter Ziehgriff zwischen zwei Spalten. Er kennt nur die Breite der
 * EINEN Spalte, die er verstellt (`value`), und meldet die neue über
 * `onChange` — Grenzen, Standardwert und Speichern gehören dem Aufrufer.
 *
 * `direction` sagt, wohin die Spalte wächst, wenn der Griff nach rechts wandert:
 * +1 für eine Spalte links vom Griff (Seitenliste), -1 für eine rechts davon
 * (Bedienung/Dateien). In Rechts-nach-links-Sprachen kehrt sich das um.
 *
 * Bedienbar mit Maus/Stift/Finger (Pointer-Capture, damit der Griff auch bei
 * schnellem Ziehen über dem Editor nicht verloren geht) und ohne Maus: Pfeiltasten
 * ±16 px, Pos1/Ende springen an die Grenzen, Doppelklick stellt die
 * Standardbreite wieder her.
 */
export const ResizeHandle: React.FC<{
  value: number;
  min: number;
  max: number;
  defaultValue: number;
  direction: 1 | -1;
  label: string;
  testId?: string;
  onChange: (value: number) => void;
}> = ({
  value,
  min,
  max,
  defaultValue,
  direction,
  label,
  testId,
  onChange,
}) => {
  const [dragging, setDragging] = useState(false);
  const start = useRef<{ x: number; value: number } | null>(null);

  // RTL spiegelt die Spalten, der Griff wandert dann mit der Maus in die
  // Gegenrichtung.
  const sign = () =>
    (document.documentElement.dir === "rtl" ? -1 : 1) * direction;

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    start.current = { x: event.clientX, value };
    setDragging(true);
    // Keine Textauswahl quer über den Editor, solange gezogen wird.
    document.body.classList.add("lv-resizing");
    event.preventDefault();
  };

  const onPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const from = start.current;
    if (!from) return;
    onChange(clamp(from.value + sign() * (event.clientX - from.x), min, max));
  };

  const stop = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    start.current = null;
    setDragging(false);
    document.body.classList.remove("lv-resizing");
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    let next: number | null = null;
    // Pfeil = Richtung, in die der Griff wandert; die Spalte wächst je nach
    // Seite mit oder gegen diese Richtung.
    if (event.key === "ArrowRight") next = value + sign() * RESIZE_KEY_STEP;
    else if (event.key === "ArrowLeft") next = value - sign() * RESIZE_KEY_STEP;
    else if (event.key === "Home") next = min;
    else if (event.key === "End") next = max;
    if (next === null) return;
    event.preventDefault();
    onChange(clamp(next, min, max));
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-label={label}
      title={label}
      tabIndex={0}
      data-testid={testId}
      data-dragging={dragging ? "true" : undefined}
      className="tts-resize"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={stop}
      onPointerCancel={stop}
      onLostPointerCapture={stop}
      onKeyDown={onKeyDown}
      onDoubleClick={() => onChange(defaultValue)}
    />
  );
};
