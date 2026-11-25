/**
 * Absatzsprünge der Wiedergabeleiste (lva#781, wie am iPhone).
 *
 * Ein Absatz beginnt am Textanfang und nach jeder Leerzeile. Zurück springt
 * an den Anfang des laufenden Absatzes; steht die Stelle schon dort (bis
 * `slack` Zeichen dahinter), geht es einen Absatz weiter zurück.
 */
export function paragraphStarts(text: string): number[] {
  const starts: number[] = [];
  const first = text.search(/\S/);
  if (first < 0) return starts;
  starts.push(first);
  // Auch Windows-Zeilenenden (\r\n) trennen Absätze (Codex-Review #788).
  const re = /\n[ \t\r]*\n\s*/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    const at = m.index + m[0].length;
    if (at < text.length && at > starts[starts.length - 1]) starts.push(at);
  }
  return starts;
}

/** Ziel eines Absatzsprungs oder `null`, wenn es keinen Absatz in der Richtung gibt. */
export function paragraphTarget(
  text: string,
  at: number,
  delta: -1 | 1,
  slack = 2,
): number | null {
  const starts = paragraphStarts(text);
  if (starts.length === 0) return null;
  let i = 0;
  while (i + 1 < starts.length && starts[i + 1] <= at) i++;
  if (delta > 0) return i + 1 < starts.length ? starts[i + 1] : null;
  if (at > starts[i] + slack) return starts[i];
  return i > 0 ? starts[i - 1] : starts[0];
}
