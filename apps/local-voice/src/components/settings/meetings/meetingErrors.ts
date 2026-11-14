import type { TFunction } from "i18next";
import { SLIDE_ERROR_KEYS } from "@/lib/meetingSlides";

/**
 * Backend error/status codes for meeting recording and import, mapped to
 * their i18n keys. The recorder's `delta_store_failed` command error comes
 * back as `"delta_store_failed: <details>"` (the store's own error message
 * is appended after a colon), while the same code arrives verbatim as an
 * event payload — so the code is normalized by taking everything before the
 * first colon before looking it up.
 *
 * Convention (#15): every error a command returns is `"<code>"` or
 * `"<code>: <detail>"` with a snake_case ASCII code. The code is the only part
 * the user ever sees (translated here); the detail is the cause for the log and
 * a bug report. Backend prose in any language (German strings of the minutes
 * run, English strings of the store) is therefore not part of the contract:
 * database and storage failures are `store_failed`, a store that did not open
 * at start-up is `meetings_unavailable`. Unknown codes still fall back to the
 * raw text, so nothing is hidden that a user might need to report.
 */
const ERROR_KEY_MAP: Record<string, string> = {
  consent_required: "meetings.errors.consentRequired",
  dictation_active: "meetings.errors.dictationActive",
  delta_store_failed: "meetings.errors.deltaStoreFailed",
  chunk_transcription_failed: "meetings.errors.chunkTranscriptionFailed",
  loopback_start_timeout: "meetings.errors.loopbackStartTimeout",
  loopback_start_failed: "meetings.errors.loopbackStartFailed",
  loopback_died: "meetings.errors.loopbackDied",
  // #537: Mitschnitt des Systemtons (YouTube); anders als in der Besprechung gibt
  // es kein Mikrofon, auf das die Aufnahme zurueckfaellt.
  playback_capture_failed: "meetings.errors.playbackCaptureFailed",
  playback_capture_timeout: "meetings.errors.playbackCaptureTimeout",
  playback_capture_died: "meetings.errors.playbackCaptureDied",
  // lva#538: Abbruch des Starts, Schreibfehler der WAV (Live-Aufnahmen auch)
  start_cancelled: "meetings.errors.startCancelled",
  start_busy: "meetings.errors.startBusy",
  // lva#538: Loeschen waehrend einer Aufnahme (nichts wurde geloescht) und fehlende Vorgangs-Id
  recording_busy: "meetings.errors.recordingBusy",
  delete_in_progress: "meetings.errors.deleteInProgress",
  invalid_operation: "meetings.errors.invalidOperation",
  audio_write_failed: "meetings.errors.audioWriteFailed",
  audio_paths_failed: "meetings.errors.storeFailed",
  mic_stream_error: "meetings.errors.micStreamError",
  import_failed: "meetings.errors.importFailed",
  already_recording: "meetings.errors.alreadyRecording",
  not_recording: "meetings.errors.notRecording",
  meeting_not_found: "meetings.errors.notFound",
  no_audio: "meetings.errors.noAudio",
  audio_missing: "meetings.errors.audioMissing",
  retranscribe_failed: "meetings.errors.retranscribeFailed",
  // M2-P2d
  final_pass_skipped: "meetings.errors.finalPassSkipped",
  // P8a
  final_pass_stopped: "meetings.errors.finalPassStopped",
  meeting_busy: "meetings.errors.meetingBusy",
  job_busy: "meetings.progress.errors.busy",
  no_job: "meetings.progress.errors.noJob",
  not_pausable: "meetings.progress.errors.notPausable",
  job_stopping: "meetings.progress.errors.stopping",
  not_cancelled: "meetings.progress.errors.notCancelled",
  transcription_failed: "meetings.errors.chunkTranscriptionFailed",
  // U7
  import_source_missing: "meetings.errors.importSourceMissing",
  import_path_invalid: "meetings.errors.importPathInvalid",
  import_panicked: "meetings.errors.importFailed",
  not_in_queue: "meetings.queue.errors.notInQueue",
  not_queued: "meetings.queue.errors.notQueued",
  // G1 (#70)
  target_not_empty: "meetings.errors.targetNotEmpty",
  // #15
  meetings_unavailable: "meetings.errors.meetingsUnavailable",
  store_failed: "meetings.errors.storeFailed",
  subtitle_unreadable: "meetings.errors.subtitleUnreadable",
  subtitle_invalid: "meetings.errors.subtitleInvalid",
  // B2: Bitte eines Ablaufs um die Einwilligung zur Aufnahme
  consent_not_pending: "meetings.errors.consentNotPending",
  consent_invalid: "meetings.errors.consentInvalid",
  // Mehrere Audiodateien je Besprechung (parts.rs)
  parts_too_few: "meetings.parts.errors.tooFew",
  parts_too_many: "meetings.parts.errors.tooMany",
  parts_not_ready: "meetings.parts.errors.notReady",
  parts_no_audio: "meetings.parts.errors.noAudio",
  parts_audio_missing: "meetings.parts.errors.audioMissing",
  parts_mixed_sources: "meetings.parts.errors.mixedSources",
  parts_has_variants: "meetings.parts.errors.hasVariants",
  parts_changed: "meetings.parts.errors.changed",
  parts_busy: "meetings.parts.errors.busy",
  parts_failed: "meetings.parts.errors.failed",
  parts_subtitle: "meetings.parts.errors.subtitle",
  parts_target_invalid: "meetings.parts.errors.targetInvalid",
  // lva#522: Bereich transkribieren und Aufnahme zuschneiden
  invalid_range: "meetings.errors.invalidRange",
  nothing_to_trim: "meetings.errors.nothingToTrim",
  trim_changed: "meetings.errors.trimChanged",
  trim_failed: "meetings.errors.trimFailed",
  metadata_unreadable: "meetings.errors.metadataUnreadable",
  rerun_pending: "meetings.errors.rerunPending",
  partial_rerun_range_damaged: "meetings.errors.rerunRangeDamaged",
  wav_unreadable: "meetings.errors.wavUnreadable",
  wav_write_failed: "meetings.errors.wavWriteFailed",
  // D4: Folienerkennung (recording_active, meeting_not_finished, slides_*)
  ...SLIDE_ERROR_KEYS,
};

/** U7: Fehlercodes beim Bearbeiten der Metadaten als i18n-Schluessel. */
const METADATA_ERROR_KEYS: Record<string, string> = {
  meeting_not_found: "meetings.metadata.errors.meetingNotFound",
  title_empty: "meetings.metadata.errors.titleEmpty",
  title_too_long: "meetings.metadata.errors.titleTooLong",
  description_too_long: "meetings.metadata.errors.descriptionTooLong",
  date_invalid: "meetings.metadata.errors.dateInvalid",
  person_not_found: "meetings.metadata.errors.personNotFound",
  folder_not_found: "meetings.metadata.errors.folderNotFound",
};

/** i18n-Schluessel eines Fehlers beim Bearbeiten der Metadaten (sonst ein allgemeiner). */
export const metadataErrorKey = (code: string): string =>
  METADATA_ERROR_KEYS[code.split(":")[0].trim()] ??
  "meetings.metadata.errors.unknown";

/**
 * Translates a raw backend error/status code into a user-facing message.
 * Unknown codes (arbitrary I/O or DB error strings that aren't part of the
 * fixed vocabulary above) fall back to the raw string rather than hiding
 * information the user might need to report a bug.
 */
export const translateMeetingError = (code: string, t: TFunction): string => {
  const normalized = code.split(":")[0].trim();
  const key = ERROR_KEY_MAP[normalized];
  return key ? t(key) : code;
};

/**
 * Text zu einem gescheiterten Loeschen (lva#538): Die beiden Codes, bei denen NICHTS geloescht
 * wurde und ein neuer Versuch sinnvoll ist (die Aufnahme wird noch beendet, es laeuft schon ein
 * Loeschen), bekommen ihre eigene Meldung; alles andere bleibt der allgemeine Text.
 */
export const deleteErrorText = (raw: string, t: TFunction): string => {
  const code = raw.split(":")[0].trim();
  return code === "recording_busy" || code === "delete_in_progress"
    ? translateMeetingError(raw, t)
    : t("meetings.errors.deleteFailed");
};
