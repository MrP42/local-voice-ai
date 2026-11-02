import type { AudioTrack, AudioVersion } from "@/bindings";

/** Reine Hilfen ohne Tauri-Abhängigkeit (laufen auch in den Node-Tests). */

/**
 * Die Teile einer Audiofassung. Altbestand ohne `tracks` hat genau einen
 * Teil (die Datei `file`), wie `tracks_of` im Kern.
 */
export function tracksOf(audio: AudioVersion | null | undefined): AudioTrack[] {
  if (!audio) return [];
  if (audio.tracks?.length) return audio.tracks;
  if (!audio.file) return [];
  return [
    {
      file: audio.file,
      title: audio.file.replace(/\.[^.]+$/, ""),
      duration_ms: audio.duration_ms,
      transcript_job: audio.transcript_job,
    },
  ];
}

/**
 * Kennungen und Technikangaben, die Hörbuch-Dateinamen mitbringen (Audible,
 * CD-Rips): ASIN („B0DD7W1VFS“), ISBN („3732455823“), Labelcode und Format
 * („LC 128 44100 Stereo“, „128kbps“, „44.1 kHz“). Sie stehen am Ende oder
 * in Klammern, nie mitten im Titel – „Fahrenheit 451“ bleibt ganz.
 */
const NOISE: RegExp[] = [
  // Klammern mit nichts als Technik oder Kennung: „[B0DD7W1VFS]“, „(128 kbps)“.
  /\s*[[(]\s*(?:B0[0-9A-Z]{8}|(?:97[89])?\d{9}[\dX]|\d{2,3}\s*kbps|LC\s*\d+[^)\]]*)\s*[)\]]/giu,
  // Labelcode samt Bitrate, Abtastrate, Kanälen: „LC 128 44100 Stereo“.
  /\s+LC\s*\d{2,5}(?:\s+\d{4,6})?(?:\s+(?:stereo|mono|joint[\s-]?stereo))?\s*$/iu,
  /\s+\d{2,3}\s*kbps\b.*$/iu,
  /\s+(?:22050|32000|44100|48000)\s*(?:hz)?(?:\s+(?:stereo|mono))?\s*$/iu,
  /\s+\d{2}(?:[.,]\d)?\s*khz(?:\s+(?:stereo|mono))?\s*$/iu,
  /\s+(?:stereo|mono)\s*$/iu,
  // ASIN und ISBN als eigenes Wort am Ende.
  /\s+B0[0-9A-Z]{8}\s*$/u,
  /\s+(?:97[89])?\d{9}[\dX]\s*$/u,
  /\s+(?:97[89]-)?\d{1,5}-\d{1,7}-\d{1,7}-[\dX]\s*$/u,
];

/** Titel ohne ASIN/ISBN- und Format-Rauschen; mehrfach, weil es sich stapelt. */
export function cleanTitle(title: string): string {
  let s = title.replace(/\s+/g, " ").trim();
  for (let round = 0; round < 4; round++) {
    const before = s;
    for (const re of NOISE) s = s.replace(re, "");
    if (s === before) break;
    s = s.replace(/[\s,;:–—-]+$/u, "").trim();
  }
  // Nur Rauschen? Dann lieber den alten Titel behalten.
  return s || title.trim();
}

/** Trägt der Titel Rauschen, das {@link cleanTitle} entfernen würde? */
export const hasTitleNoise = (title: string) =>
  cleanTitle(title) !== title.replace(/\s+/g, " ").trim();

/**
 * Titel und Autor aus einem Dateinamen raten: „Theodor Fontane - Effi
 * Briest.m4b“, „Effi_Briest.epub“. Nur ein Vorschlag, der Dialog zeigt ihn an.
 */
export function guessFromFileName(path: string): {
  title: string;
  authors: string[];
} {
  const base = cleanTitle(
    (path.split(/[\\/]/).pop() ?? "")
      .replace(/\.[^.]+$/, "")
      .replace(/[_]+/g, " ")
      .replace(/\s+/g, " ")
      .trim(),
  );
  const parts = base.split(/\s+[-–]\s+/);
  if (parts.length === 2) {
    const [a, b] = parts;
    // Der kürzere Teil mit zwei bis drei Wörtern ist eher der Autor.
    const looksLikeName = (s: string) => /^\S+(\s+\S+){1,2}$/.test(s);
    if (looksLikeName(a) && !looksLikeName(b))
      return { title: b, authors: [a] };
    if (looksLikeName(b) && !looksLikeName(a))
      return { title: a, authors: [b] };
    return { title: b, authors: [a] };
  }
  return { title: base, authors: [] };
}
