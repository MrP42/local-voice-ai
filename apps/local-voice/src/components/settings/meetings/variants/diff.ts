import type { StoredSegment } from "@/bindings";

/** Ein Stueck des wortweisen Vergleichs von Fassung A nach Fassung B. */
export interface DiffOp {
  /** `same`: in beiden; `del`: nur in A (Loeschung); `ins`: nur in B (Einfuegung). */
  kind: "same" | "del" | "ins";
  text: string;
}

/** Eine Zeile der Vergleichsansicht: ein Segment von A und der passende Text aus B. */
export interface DiffRow {
  startMs: number;
  ops: DiffOp[];
  /** Die Zeile unterscheidet sich (mindestens eine Einfuegung oder Loeschung). */
  changed: boolean;
}

/** Hoechstzahl Tabellenzellen der Zeilen-LCS; darueber wird nicht fein verglichen. */
const MAX_CELLS = 4_000_000;

const tokens = (text: string) => text.split(/\s+/).filter(Boolean);

/** Gleichheit ohne Gross-/Kleinschreibung und ohne Satzzeichen am Rand. */
const key = (word: string) =>
  word.toLowerCase().replace(/^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu, "");

const push = (ops: DiffOp[], kind: DiffOp["kind"], word: string) => {
  const last = ops[ops.length - 1];
  if (last && last.kind === kind) last.text += ` ${word}`;
  else ops.push({ kind, text: word });
};

/** Wortweiser Vergleich (laengste gemeinsame Teilfolge). */
export function wordDiff(a: string, b: string): DiffOp[] {
  const x = tokens(a);
  const y = tokens(b);
  const ops: DiffOp[] = [];
  if (x.length * y.length > MAX_CELLS) {
    if (x.length) ops.push({ kind: "del", text: x.join(" ") });
    if (y.length) ops.push({ kind: "ins", text: y.join(" ") });
    return ops;
  }
  const kx = x.map(key);
  const ky = y.map(key);
  const w = y.length + 1;
  const table = new Uint32Array((x.length + 1) * w);
  for (let i = x.length - 1; i >= 0; i--) {
    for (let j = y.length - 1; j >= 0; j--) {
      table[i * w + j] =
        kx[i] === ky[j]
          ? table[(i + 1) * w + j + 1] + 1
          : Math.max(table[(i + 1) * w + j], table[i * w + j + 1]);
    }
  }
  let i = 0;
  let j = 0;
  while (i < x.length && j < y.length) {
    if (kx[i] === ky[j]) {
      push(ops, "same", y[j]);
      i++;
      j++;
    } else if (table[(i + 1) * w + j] >= table[i * w + j + 1]) {
      push(ops, "del", x[i++]);
    } else {
      push(ops, "ins", y[j++]);
    }
  }
  while (i < x.length) push(ops, "del", x[i++]);
  while (j < y.length) push(ops, "ins", y[j++]);
  return ops;
}

const byStart = (s: StoredSegment[]) =>
  [...s].sort((p, q) => p.start_ms - q.start_ms);

/**
 * Legt die Segmente von B auf die von A (nach Zeit: das A-Segment mit der
 * groessten Ueberlappung, sonst das naechste) und vergleicht je Zeile wortweise.
 * Ganze Texte gegeneinander zu rechnen waere bei einer Stunde Video zu teuer und
 * liesse sich nicht lesen; so bleibt jede Zeile ein kurzes Stueck mit Zeitmarke.
 */
export function alignDiff(a: StoredSegment[], b: StoredSegment[]): DiffRow[] {
  const base = byStart(a);
  if (base.length === 0) return [];
  const buckets: string[][] = base.map(() => []);
  // Beide Seiten sind nach Start sortiert: der Zeiger `p` wandert nur vorwaerts,
  // gepruft werden die Nachbarn (linear statt quadratisch).
  let p = 0;
  for (const seg of byStart(b)) {
    while (p + 1 < base.length && base[p].end_ms < seg.start_ms) p++;
    const lo = Math.max(0, p - 2);
    const hi = Math.min(base.length - 1, p + 3);
    let best = lo;
    let bestScore = -Infinity;
    for (let index = lo; index <= hi; index++) {
      const candidate = base[index];
      const overlap =
        Math.min(candidate.end_ms, seg.end_ms) -
        Math.max(candidate.start_ms, seg.start_ms);
      // Ohne Ueberlappung zaehlt der Abstand der Mitten (negativ, je naeher desto besser).
      const score =
        overlap > 0
          ? overlap
          : -Math.abs(
              (candidate.start_ms + candidate.end_ms) / 2 -
                (seg.start_ms + seg.end_ms) / 2,
            );
      if (score > bestScore) {
        bestScore = score;
        best = index;
      }
    }
    buckets[best].push(seg.text);
  }
  return base.map((segment, index) => {
    const ops = wordDiff(segment.text, buckets[index].join(" "));
    return {
      startMs: segment.start_ms,
      ops,
      changed: ops.some((op) => op.kind !== "same"),
    };
  });
}
