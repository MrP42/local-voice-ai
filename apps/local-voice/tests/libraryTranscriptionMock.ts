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
        const books = await w.__TAURI_INTERNALS__.invoke("library_list");
        const current = books.find((b: any) => b.id === id);
        if (!current?.audio) throw "book_not_found";
        const a = args.adoption;
        if (a.append_page && (current.text?.page_id !== a.append_page || current.text.origin !== "transcript"))
          throw "stale_audiobook_completion";
        const text = { page_id: a.page_id, origin: "transcript", source_name: current.audio.file };
        const separate = current.text && !a.append_page;
        const edition = { id: `ed_${a.page_id}`, lang: current.language ?? "", title: a.edition_title,
          text, audio: null, audio_paths: [], origin: "import", added_ms: 1, updated: 1 };
        const editions = current.editions?.length ? current.editions : [
          { id: `ed_${id}`, text: current.text, audio: current.audio },
        ];
        const book = { ...current, text: separate ? current.text : text,
          editions: separate ? [...editions, edition] : current.editions,
          audio: { ...current.audio, transcript_job: null, sync: separate ? current.audio.sync : a.sync,
            tracks: current.audio.tracks?.map((t: any) => ({ ...t, transcript_job: null, transcript_jobs: [] })) } };
        const saved = await w.__TAURI_INTERNALS__.invoke("library_save", { book });
        claims.delete(id);
        return saved;
      }
      if (claims.has(id)) throw "audiobook_adopting";
      if (cmd === "library_transcription_remove") {
        const books = await w.__TAURI_INTERNALS__.invoke("library_list");
        const current = books.find((b: any) => b.id === id);
        for (const job of args.jobIds) {
          const status = await w.__TAURI_INTERNALS__.invoke("meetings_get_status", { meetingId: job });
          if (status !== null && !["ready", "done", "failed", "cancelled"].includes(status)) throw "job_stopping";
        }
        const unlink = (audio: any) => audio ? { ...audio,
          transcript_job: args.jobIds.includes(audio.transcript_job) ? null : audio.transcript_job,
          tracks: audio.tracks?.map((t: any) => ({ ...t,
            transcript_job: args.jobIds.includes(t.transcript_job) ? null : t.transcript_job,
            transcript_jobs: t.transcript_jobs?.filter((p: any) => !args.jobIds.includes(p.job)) })) } : audio;
        await w.__TAURI_INTERNALS__.invoke("library_save", { book: { ...current, audio: unlink(current.audio) } });
        for (const job of args.jobIds)
          await w.__TAURI_INTERNALS__.invoke("meetings_delete", { meetingId: job });
        return null;
      }
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
