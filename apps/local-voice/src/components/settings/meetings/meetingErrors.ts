import type { TFunction } from "i18next";

/**
 * Backend error/status codes for meeting recording and import, mapped to
 * their i18n keys. The recorder's `delta_store_failed` command error comes
 * back as `"delta_store_failed: <details>"` (the store's own error message
 * is appended after a colon), while the same code arrives verbatim as an
 * event payload — so the code is normalized by taking everything before the
 * first colon before looking it up.
 */
const ERROR_KEY_MAP: Record<string, string> = {
  consent_required: "meetings.errors.consentRequired",
  dictation_active: "meetings.errors.dictationActive",
  delta_store_failed: "meetings.errors.deltaStoreFailed",
  chunk_transcription_failed: "meetings.errors.chunkTranscriptionFailed",
  loopback_start_timeout: "meetings.errors.loopbackStartTimeout",
  loopback_start_failed: "meetings.errors.loopbackStartFailed",
  loopback_died: "meetings.errors.loopbackDied",
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
};

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
