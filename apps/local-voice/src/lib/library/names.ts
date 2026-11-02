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
 * Titel und Autor aus einem Dateinamen raten: „Theodor Fontane - Effi
 * Briest.m4b“, „Effi_Briest.epub“. Nur ein Vorschlag, der Dialog zeigt ihn an.
 */
export function guessFromFileName(path: string): {
  title: string;
  authors: string[];
} {
  const base = (path.split(/[\\/]/).pop() ?? "")
    .replace(/\.[^.]+$/, "")
    .replace(/[_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
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
