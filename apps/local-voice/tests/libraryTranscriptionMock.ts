import type { Page } from "@playwright/test";

/** Buchsteuerung fuer die Bibliotheks-Attrappen, ohne echte Auftraege. */
export async function installLibraryTranscriptionMock(page: Page) {
  await page.addInitScript(() => {
    const states = new Map<string, string>();
    const generations = new Map<string, number>();
    const claims = new Map<string, string>();
    const w = window as unknown as Record<string, any>;
    w.__libraryTranscriptionMock = async (
      cmd: string,
      args: Record<string, any> = {},
    ) => {
      const id = String(args.bookId ?? args.book?.id);
      const state = states.get(id) ?? "running";
      if (cmd === "library_transcription_status") {
        const books = await w.__TAURI_INTERNALS__.invoke("library_list");
        const book = books.find((b: any) => b.id === id);
        const tracks = book?.audio?.tracks?.length
          ? book.audio.tracks : [book?.audio];
        const ids = [...new Set<string>(tracks.flatMap((t: any) =>
          t?.transcript_jobs?.length
            ? t.transcript_jobs.map((p: any) => p.job)
            : t?.transcript_job ? [t.transcript_job] : [],
        ))].sort();
        return {
          book_id: id,
          state,
          generations: ids.map((job) => [job, generations.get(id) ?? 0]),
        };
      }
      if (cmd === "library_transcription_claim") {
        const current = await w.__libraryTranscriptionMock("library_transcription_status", { bookId: id });
        if (state !== "running" || claims.has(id) || !current.generations.length ||
            JSON.stringify(args.generations) !== JSON.stringify(current.generations))
          throw "stale_audiobook_completion";
        const token = `synthetic-claim-${id}`;
        claims.set(id, token);
        return token;
      }
      if (cmd === "library_transcription_release") {
        if (claims.get(id) === args.token) claims.delete(id);
        return null;
      }
      if (cmd === "library_transcription_finish") {
        if (claims.get(id) !== args.token) throw "stale_audiobook_completion";
        const saved = await w.__TAURI_INTERNALS__.invoke("library_save", { book: args.book });
        claims.delete(id);
        return saved;
      }
      if (claims.has(id)) throw "audiobook_adopting";
      if (cmd === "library_transcription_pause") states.set(id, "paused");
      else if (cmd === "library_transcription_resume") states.set(id, "running");
      else if (cmd === "library_transcription_cancel") {
        states.set(id, "cancelled");
        if (!args.keepPartial) generations.set(id, (generations.get(id) ?? 0) + 1);
      } else throw "unknown_library_transcription_command";
      return null;
    };
  });
}
