import type { Einstufung, EinstufungsErgebnis } from "@/bindings";

/** Ad-hoc wird bewusst als unbekannte Runde bestätigt, niemals als intern. */
export const aiReleaseConfirmation = (
  result: Pick<EinstufungsErgebnis, "aus_termin" | "einstufung">,
): Einstufung | null =>
  !result.aus_termin && result.einstufung !== "extern"
    ? "unbekannt"
    : result.einstufung === "unbekannt"
      ? null
      : result.einstufung;

const OPEN_EVENT = "lva:open-meeting-ai-release";

/** Öffnet die Freigabe in den Details der aktuell angezeigten Aufnahme. */
export const requestOpenMeetingAiRelease = (meetingId: string) => {
  window.dispatchEvent(new CustomEvent(OPEN_EVENT, { detail: meetingId }));
};

export const onOpenMeetingAiRelease = (
  meetingId: string,
  open: () => void,
) => {
  const handler = (event: Event) => {
    if ((event as CustomEvent<unknown>).detail === meetingId) open();
  };
  window.addEventListener(OPEN_EVENT, handler);
  return () => window.removeEventListener(OPEN_EVENT, handler);
};
