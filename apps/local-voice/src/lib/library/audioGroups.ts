import type { LibraryBook } from "@/bindings";
import { guessFromFileName, tracksOf } from "./names.ts";

/**
 * Mehrteilige Hörbücher erkennen (lva#204). Ein Hörbuch auf CD besteht aus
 * Dutzenden Dateien („… Disc 1 - 01 - Track 01.mp3“); sie gehören in **ein**
 * Buch. Zwei Fälle:
 * - {@link groupAudioFiles}: beim Import werden Dateien nach Ordner und
 *   Titelstamm gruppiert, je Gruppe entsteht ein Buch;
 * - {@link splitAudiobookGroups}: schon als einzelne Bücher importierte Teile
 *   werden als Kandidaten für das Zusammenführen gefunden.
 */

export interface AudioGroup {
  /** Ordner (mit `/`) und gefalteter Titelstamm. */
  key: string;
  dir: string;
  title: string;
  authors: string[];
  /** In Hörreihenfolge: Disc, dann Track, dann Name. */
  paths: string[];
}

export interface SplitGroup {
  key: string;
  /** Titel ohne Disc-/Track-Angaben. */
  title: string;
  authors: string[];
  /** In Hörreihenfolge. */
  books: LibraryBook[];
}

const NOTE =
  "(?:disc|disk|cd|teil|part|pt|kapitel|kap|chapter|ch|track|titel|spur|folge|episode)";
/** „Disc 1“, „Track 01“, „Teil 3 von 12“, „CD-2“. */
const MARKER = new RegExp(
  `(?:^|[^\\p{L}\\p{N}])${NOTE}\\.?[\\s_#-]*\\d+(?:\\s*(?:von|of|/)\\s*\\d+)?(?![\\p{L}\\p{N}])`,
  "giu",
);
const DISC = /(?:^|[^\p{L}])(?:disc|disk|cd)\.?[\s_#-]*(\d+)/iu;
const TRACK = /(?:^|[^\p{L}])(?:track|titel|spur)\.?[\s_#-]*(\d+)/iu;
const PART =
  /(?:^|[^\p{L}])(?:teil|part|pt|kapitel|kap|chapter|ch|folge|episode)\.?[\s_#-]*(\d+)/iu;
const SEPARATOR = /\s+[-–—]\s+/;

const collator = new Intl.Collator("de", {
  numeric: true,
  sensitivity: "base",
});

const fold = (s: string) =>
  s
    .toLocaleLowerCase("de")
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();

const normalize = (path: string) => path.replace(/\\/g, "/");
const dirOf = (path: string) => {
  const p = normalize(path);
  const at = p.lastIndexOf("/");
  return at >= 0 ? p.slice(0, at) : "";
};
const baseOf = (path: string) => {
  const p = normalize(path);
  return p.slice(p.lastIndexOf("/") + 1).replace(/\.[^.]+$/, "");
};

interface Analysis {
  /** Titel (und Autor) ohne Nummern, als Dateiname-Vorschlag. */
  stem: string;
  /** Mindestens ein Disc-/Track-/Teil-Marker oder eine reine Nummer. */
  numbered: boolean;
  disc: number;
  track: number;
}

const num = (re: RegExp, s: string) => {
  const m = re.exec(s);
  return m ? Number(m[1]) : null;
};

/**
 * Name zerlegen: Marker („Disc 1“, „Track 01“) und Segmente, die nur aus einer
 * Nummer bestehen, fallen weg. Bleibt danach eine führende oder abschließende
 * Nummer am Stamm hängen, fällt auch sie weg – aber nur, wenn der Name schon
 * einen Marker oder ein Nummernsegment hatte („Fahrenheit 451“ bleibt ganz).
 */
export function analyzeName(name: string): Analysis {
  const cleaned = name.replace(/_+/g, " ").replace(/\s+/g, " ").trim();
  const disc = num(DISC, cleaned) ?? 0;
  const segmentsRaw = cleaned.split(SEPARATOR);
  let numbered = false;
  let bareTrack: number | null = null;
  const segments: string[] = [];
  for (const raw of segmentsRaw) {
    let seg = raw;
    if (MARKER.test(seg)) numbered = true;
    MARKER.lastIndex = 0;
    seg = seg.replace(MARKER, " ");
    seg = seg
      .replace(/\(\s*\)|\[\s*\]/g, " ")
      .replace(/\s+/g, " ")
      .trim();
    if (/^[\s\d.)(\-–—]*$/.test(seg)) {
      // Nur eine Nummer (oder nichts): „01“.
      if (/\d/.test(seg) && bareTrack === null)
        bareTrack = Number(seg.replace(/\D/g, ""));
      if (/\d/.test(seg)) numbered = true;
      continue;
    }
    segments.push(seg);
  }
  if (numbered) {
    // Führende („01 Titel“, „01. Titel“) und abschließende Nummern.
    const last = segments.length - 1;
    if (segments.length) {
      const lead = /^(\d{1,3})[\s.)_-]+(?=\S)/.exec(segments[0]);
      if (lead) {
        if (bareTrack === null) bareTrack = Number(lead[1]);
        segments[0] = segments[0].slice(lead[0].length);
      }
      const trail = /(?<=\S)[\s._(-]+(\d{1,3})\)?$/.exec(segments[last]);
      if (trail) {
        if (bareTrack === null) bareTrack = Number(trail[1]);
        segments[last] = segments[last].slice(0, trail.index);
      }
    }
  }
  const stem = segments
    .map((s) => s.replace(/^[\s.\-–—,;:]+|[\s.\-–—,;:]+$/g, ""))
    .filter(Boolean)
    .join(" - ");
  const leading = /^(\d{1,3})[\s.)_-]+\S/.exec(cleaned);
  const track =
    num(TRACK, cleaned) ??
    num(PART, cleaned) ??
    bareTrack ??
    (leading ? Number(leading[1]) : 0);
  return { stem, numbered, disc, track };
}

/** Ordner, in dem die Teile liegen: „CD 1“, „Disc 2“ am Ende zählen nicht. */
function groupDir(dir: string): { dir: string; disc: number } {
  const parts = dir.split("/");
  let disc = 0;
  while (parts.length > 1) {
    const last = parts[parts.length - 1];
    if (!/^\s*(?:disc|disk|cd)\.?[\s_#-]*\d+\s*$/i.test(last)) break;
    disc = disc || num(DISC, last) || 0;
    parts.pop();
  }
  return { dir: parts.join("/"), disc };
}

const nameFromStem = (stem: string) => {
  const g = guessFromFileName(`${stem}.mp3`);
  return { title: g.title, authors: g.authors };
};

interface FileInfo {
  path: string;
  dir: string;
  name: string;
  stem: string;
  disc: number;
  track: number;
  numbered: boolean;
}

function byListening<T extends { disc: number; track: number; name: string }>(
  a: T,
  b: T,
): number {
  return (
    a.disc - b.disc || a.track - b.track || collator.compare(a.name, b.name)
  );
}

/**
 * Audiodateien nach Ordner und Titelstamm gruppieren, je Gruppe ein Buch.
 * Die Reihenfolge der Gruppen folgt dem ersten Auftreten ihrer Dateien.
 */
export function groupAudioFiles(paths: string[]): AudioGroup[] {
  const infos: FileInfo[] = paths.map((path) => {
    const own = groupDir(dirOf(path));
    const a = analyzeName(baseOf(path));
    return {
      path,
      dir: own.dir,
      name: baseOf(path),
      stem: a.stem,
      disc: a.disc || own.disc,
      track: a.track,
      numbered: a.numbered,
    };
  });
  const buckets = new Map<string, FileInfo[]>();
  for (const info of infos) {
    // Ohne Stamm („Track 01.mp3“) gehören alle Dateien des Ordners zusammen.
    const key = `${info.dir}\u0000${fold(info.stem)}`;
    const list = buckets.get(key) ?? [];
    list.push(info);
    buckets.set(key, list);
  }
  // Nummerierte Dateien mit eigenen Titeln („01 Einleitung“, „02 Prolog“)
  // eines Ordners sind Kapitel eines Buchs, nicht viele Bücher.
  const byDir = new Map<string, string[]>();
  for (const [key, list] of buckets) {
    const dir = list[0].dir;
    byDir.set(dir, [...(byDir.get(dir) ?? []), key]);
  }
  const chapterKeys = new Set<string>();
  for (const [dir, keys] of byDir) {
    const lists = keys.map((k) => buckets.get(k)!);
    const tracks = lists.map((l) => l[0].track).sort((x, y) => x - y);
    const chapters =
      keys.length >= 3 &&
      lists.every((l) => l.length === 1 && l[0].track > 0) &&
      // Lückenlos aufsteigend: 1, 2, 3 … (mit Platz für eine fehlende Nummer).
      tracks.every(
        (t, i) => i === 0 || (t > tracks[i - 1] && t - tracks[i - 1] <= 2),
      );
    if (!chapters) continue;
    const merged = lists.flat();
    keys.forEach((k) => buckets.delete(k));
    buckets.set(`${dir}\u0000`, merged);
    chapterKeys.add(`${dir}\u0000`);
  }
  const groups: AudioGroup[] = [];
  for (const [key, list] of buckets) {
    const files = [...list].sort(byListening);
    const first = files[0];
    // Mit Stamm: Titel und Autor daraus; ohne: Ordnername, sonst Dateiname.
    const folder = first.dir.split("/").pop() ?? "";
    const base = chapterKeys.has(key)
      ? folder || first.name
      : first.stem || folder || first.name;
    let { title, authors } = nameFromStem(base);
    if (!title) title = first.name;
    // Ein einzelnes Teil behält seinen vollen Namen („Fahrenheit 451“).
    if (files.length === 1 && !first.numbered) {
      ({ title, authors } = guessFromFileName(first.path));
    }
    groups.push({
      key,
      dir: first.dir,
      title,
      authors,
      paths: files.map((f) => f.path),
    });
  }
  // Reihenfolge des ersten Auftretens in `paths`.
  const order = new Map(paths.map((p, i) => [p, i]));
  return groups.sort(
    (a, b) =>
      Math.min(...a.paths.map((p) => order.get(p) ?? 0)) -
      Math.min(...b.paths.map((p) => order.get(p) ?? 0)),
  );
}

const surname = (author: string | undefined) =>
  fold(author?.split(/\s+/).pop() ?? "");

/**
 * Bücher mit Audio, die wie die Teile **eines** Hörbuchs aussehen: zwei oder
 * mehr mit gleichem Stamm (aus Titel, sonst Teiltitel) und ohne Textfassung.
 * Ein Buch mit Disc-/Track-Angabe im Titel zählt, ein bloßes „Harry Potter 2“
 * nicht – das wären zwei Bücher einer Reihe.
 */
export function splitAudiobookGroups(books: LibraryBook[]): SplitGroup[] {
  interface Member {
    book: LibraryBook;
    a: Analysis;
    name: ReturnType<typeof nameFromStem>;
    order: number;
  }
  const buckets = new Map<string, Member[]>();
  books.forEach((book, index) => {
    if (!book.audio || book.text) return;
    const tracks = tracksOf(book.audio);
    if (!tracks.length) return;
    let a = analyzeName(book.title);
    if (!a.stem && tracks.length === 1) a = analyzeName(tracks[0].title);
    if (!a.stem) return;
    const name = nameFromStem(a.stem);
    const author = book.authors[0] ?? name.authors[0];
    const key = `${fold(name.title)}|${surname(author)}`;
    const list = buckets.get(key) ?? [];
    list.push({ book, a, name, order: index });
    buckets.set(key, list);
  });
  const groups: SplitGroup[] = [];
  for (const [key, members] of buckets) {
    // Mindestens zwei, und mindestens zwei davon tragen eine Nummerierung;
    // fertig zusammengeführte Bücher (mehrere Teile) dürfen dabei sein.
    const numbered = members.filter(
      (m) => m.a.numbered || tracksOf(m.book.audio).length > 1,
    );
    if (members.length < 2 || numbered.length < 2) continue;
    const sorted = [...members].sort(
      (x, y) =>
        x.a.disc - y.a.disc ||
        x.a.track - y.a.track ||
        collator.compare(x.book.title, y.book.title),
    );
    const withAuthor = sorted.find((m) => m.book.authors.length);
    groups.push({
      key,
      title: sorted[0].name.title,
      authors: withAuthor?.book.authors ?? sorted[0].name.authors,
      books: sorted.map((m) => m.book),
    });
  }
  return groups.sort(
    (a, b) =>
      Math.min(...a.books.map((x) => books.indexOf(x))) -
      Math.min(...b.books.map((x) => books.indexOf(x))),
  );
}
