import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (file) => readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
const hook = read("src/components/library/useLibrary.ts");
const command = read("src-tauri/src/commands/library_transcription.rs");
const queue = read("src-tauri/src/managers/meetings/queue.rs");

// Vertragspruefungen ergaenzen die Verhaltenstests in Rust; kein Ersatz fuer tsc/cargo.
function counterprobe(source, verify, from, to) {
  verify(source);
  assert.ok(source.includes(from), "Gegenprobe muss die Korrektur treffen");
  assert.throws(() => verify(source.replace(from, to)));
}

test("adoption sends a transcript payload instead of an incomplete Edition", () => {
  counterprobe(hook, (s) => {
    assert.match(s, /libraryTranscriptionFinish\(bookId, \{/);
    assert.doesNotMatch(s, /editions: ownText/);
  }, "libraryTranscriptionFinish(bookId, {", "libraryTranscriptionFinish({");
});

test("removal delegates unlink and cleanup to one recoverable backend command", () => {
  const remove = hook.slice(hook.indexOf("const removeTranscript"), hook.indexOf("// Fortschritt je Buch"));
  counterprobe(remove, (s) => {
    assert.match(s, /libraryTranscriptionRemove\(bookId, ids\)/);
    assert.doesNotMatch(s, /meetingsDelete|await save\(/);
  }, "libraryTranscriptionRemove(bookId, ids)", "meetingsDelete(ids[0])");
});

test("backend adoption loads the current book under the library lock", () => {
  const finish = command.slice(command.indexOf("pub fn library_transcription_finish"), command.indexOf("pub fn library_transcription_claim"));
  counterprobe(finish, (s) => {
    assert.match(s, /lock\(\)[\s\S]*library_list[\s\S]*audiobook_check_claim[\s\S]*apply_transcript/);
    assert.doesNotMatch(s, /book: crate::library::LibraryBook/);
  }, "apply_transcript", "replace_book");
});

test("book cancellation requests stop before attempting its write", () => {
  const control = queue.slice(queue.indexOf("pub fn control_audiobooks"), queue.indexOf("pub fn new(deps"));
  counterprobe(control, (s) => {
    assert.match(s, /request_audiobook_stop[\s\S]*audiobook_set_state_active/);
    assert.match(s, /pending_cancels/);
  }, "request_audiobook_stop", "stop_after_persistence");
});

test("deferred yields retry only Busy", () => {
  counterprobe(queue, (s) => {
    assert.match(s, /fn finish_pending_yield[\s\S]*is_busy_error/);
    assert.match(s, /finish_pending_yield\(&store, &id/);
  }, "fn finish_pending_yield", "fn retry_all_errors");
});
