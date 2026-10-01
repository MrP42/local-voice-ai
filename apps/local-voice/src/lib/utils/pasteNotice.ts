/**
 * Which texts the "text was not inserted" notice shows.
 *
 * `partial` marks a run that inserted continuously (sentence mode, live
 * injection) and kept back only the REST: the earlier part is in the target
 * window, so "the full text is in the clipboard" would be wrong and the user
 * would paste a fragment believing it is the whole dictation.
 */
export interface PasteNoticeKeys {
  /** Title of the overlay notice. */
  title: string;
  /** Line that says where the text is now. */
  action: string;
}

export function pasteNoticeKeys(
  partial: boolean | undefined,
  transcriptInClipboard: boolean,
): PasteNoticeKeys {
  if (partial) {
    return {
      title: "overlay.notice.titlePartial",
      action: transcriptInClipboard
        ? "overlay.notice.partialInClipboard"
        : "overlay.notice.partialInHistory",
    };
  }
  return {
    title: "overlay.notice.title",
    action: transcriptInClipboard
      ? "overlay.notice.inClipboard"
      : "overlay.notice.inHistory",
  };
}
