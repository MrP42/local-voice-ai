/**
 * Hinweis nach einem Sprachwechsel oder bei einem Konflikt (lva#344, sf-06,
 * sf-07, sf-11). Eigener Speicher statt eines Toast-Pakets, weil die Regeln
 * genau festliegen: Der Hinweis bleibt 10 Sekunden stehen, die Zeit hält an,
 * solange der Mauszeiger darüber steht oder der Fokus darin liegt, und sie
 * läuft danach mit dem Rest weiter. Der Speicher überlebt den Neuaufbau der
 * Hörleiste beim Wechsel (die Leiste bekommt dabei ein neues Element).
 */

export type NoticeTone = "info" | "warn" | "alert";

export interface NoticeAction {
  id: string;
  label: string;
  run(): void | Promise<void>;
}

export interface Notice {
  id: number;
  /** `library` (Hörleiste) oder `reader` (Text-Leser). */
  owner: "library" | "reader";
  tone: NoticeTone;
  /** Text; `**fett**` wird hervorgehoben. */
  text: string;
  actions: NoticeAction[];
  /** Ohne Ablauf stehen lassen (Frage an den Nutzer). */
  sticky: boolean;
}

/** So lange bleibt ein Hinweis stehen (ms). */
export const NOTICE_MS = 10_000;

type Listener = () => void;

let current: Notice | null = null;
let counter = 0;
const listeners = new Set<Listener>();
let timer: ReturnType<typeof setTimeout> | null = null;
let remaining = NOTICE_MS;
let startedAt = 0;
const holds = new Set<string>();

const emit = () => listeners.forEach((l) => l());

function stop() {
  if (timer !== null) clearTimeout(timer);
  timer = null;
}

function arm() {
  stop();
  if (!current || current.sticky || holds.size > 0) return;
  startedAt = Date.now();
  const id = current.id;
  timer = setTimeout(() => {
    if (current?.id === id) dismissNotice(id);
  }, remaining);
}

export function showNotice(
  notice: Omit<Notice, "id" | "sticky"> & { sticky?: boolean },
): number {
  stop();
  counter += 1;
  current = { ...notice, sticky: notice.sticky ?? false, id: counter };
  remaining = NOTICE_MS;
  holds.clear();
  arm();
  emit();
  return counter;
}

export function dismissNotice(id?: number): void {
  if (id !== undefined && current?.id !== id) return;
  stop();
  current = null;
  holds.clear();
  emit();
}

/** Zeit anhalten (`hover` oder `focus`), solange mindestens ein Grund besteht. */
export function holdNotice(reason: "hover" | "focus"): void {
  if (!current || holds.has(reason)) return;
  if (holds.size === 0 && timer !== null) {
    remaining = Math.max(0, remaining - (Date.now() - startedAt));
    stop();
  }
  holds.add(reason);
}

/** Grund aufheben; ohne weiteren Grund läuft die Restzeit weiter. */
export function releaseNotice(reason: "hover" | "focus"): void {
  if (!holds.delete(reason)) return;
  if (holds.size === 0) arm();
}

export const getNotice = (): Notice | null => current;

export function subscribeNotice(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
