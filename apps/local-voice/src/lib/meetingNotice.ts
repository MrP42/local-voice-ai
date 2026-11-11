import type {
  MeetingNoticeSettings,
  NoticeApp,
  NoticeLang,
  NoticeRecord,
} from "@/bindings";

/**
 * Hinweis an Teilnehmende (G2, #497): Vorgaben und kleine Hilfen der
 * Oberfläche. Die Regeln selbst (Textbausteine, Pflicht, Nachweis) liegen im
 * Backend (`managers::compliance::notice`); hier steht nur, was die Anzeige
 * braucht. Die Vorgaben spiegeln `MeetingNoticeSettings::default()`.
 */

export const DEFAULT_MEETING_NOTICE: MeetingNoticeSettings = {
  enabled: true,
  required: false,
  export_line: true,
  default_app: "neutral",
  default_lang: "de",
  purpose: "",
  retention: "",
  templates: [],
};

export const NOTICE_APPS: NoticeApp[] = ["teams", "webex", "neutral"];
export const NOTICE_LANGS: NoticeLang[] = ["de", "en"];

/** Frist in Sekunden, in der der Nutzer das Chatfeld anklickt (Backend: 5, höchstens 15). */
export const PASTE_COUNTDOWN_SECS = 5;

/** Einstellungen mit Vorgaben aufgefüllt (ältere settings.json kennt den Schlüssel nicht). */
export const noticeSettingsOf = (
  raw: Partial<MeetingNoticeSettings> | null | undefined,
): MeetingNoticeSettings => ({ ...DEFAULT_MEETING_NOTICE, ...(raw ?? {}) });

/** Gilt die Pflicht (Funktion an UND Pflicht an)? Wie `notice::start_gate`. */
export const noticeIsRequired = (settings: MeetingNoticeSettings): boolean =>
  settings.enabled && settings.required;

/** Fehlercode eines Befehls: alles vor dem ersten Doppelpunkt. */
export const noticeErrorCode = (error: string): string =>
  error.split(":")[0].trim();

/** Der jüngste Nachweis (die Liste ist nach Zeit geordnet, älteste zuerst). */
export const latestRecord = (
  records: readonly NoticeRecord[],
): NoticeRecord | null =>
  records.length > 0 ? records[records.length - 1] : null;

/** Uhrzeit eines Nachweises ("14:32") in der Ortszeit. */
export const noticeTime = (at: number, locale?: string): string =>
  new Date(at * 1000).toLocaleTimeString(locale, {
    hour: "2-digit",
    minute: "2-digit",
  });

/** Ersetzt oder ergänzt den angepassten Baustein für App und Sprache; leerer Text entfernt ihn. */
export const withTemplate = (
  settings: MeetingNoticeSettings,
  app: NoticeApp,
  lang: NoticeLang,
  text: string,
): MeetingNoticeSettings => {
  const others = settings.templates.filter(
    (t) => !(t.app === app && t.lang === lang),
  );
  const trimmed = text.trim();
  return {
    ...settings,
    templates: trimmed === "" ? others : [...others, { app, lang, text }],
  };
};
