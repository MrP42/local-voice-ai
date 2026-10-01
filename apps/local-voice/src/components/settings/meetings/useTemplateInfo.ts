import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  commands,
  events,
  type AutoTemplateInfo,
  type MinutesMeta,
  type TemplateInfo,
} from "@/bindings";
import { AUTO_TEMPLATE_ID, DEFAULT_TEMPLATE_ID } from "@/lib/meetingNotes";

interface Loaded {
  /** Gemerkte Wahl der Besprechung (`null` = Standardvorlage). */
  choice: string | null;
  templates: TemplateInfo[];
  auto: AutoTemplateInfo | null;
  meta: MinutesMeta | null;
  /** Wohin das Protokoll automatisch abgelegt wurde. */
  file: string | null;
}

const EMPTY: Loaded = {
  choice: null,
  templates: [],
  auto: null,
  meta: null,
  file: null,
};

/**
 * Vorlage und Ablage des Protokolls einer Besprechung (G4, #70): Chip "Vorlage:
 * X" im Kopf und Zeilen im Info-Dialog. Vorher standen Pfad und "Erzeugt mit der
 * Vorlage ..." im Reiter Protokoll und kosteten dort Platz vor dem Inhalt.
 *
 * Lädt beim Wechsel der Besprechung und nach jedem fertigen Lauf (Notizen,
 * Protokoll); `reload` nach einer Änderung der Vorlagenwahl.
 */
export function useTemplateInfo(meetingId: string) {
  const { t } = useTranslation();
  const [data, setData] = useState<Loaded>(EMPTY);
  const alive = useRef(true);
  const seq = useRef(0);

  const reload = useCallback(async () => {
    const mine = ++seq.current;
    const [choice, templates, auto, meta, file] = await Promise.all([
      commands.meetingsGetTemplate(meetingId),
      commands.meetingTemplatesList(),
      commands.meetingsGetAutoTemplate(meetingId),
      commands.meetingsMinutesMeta(meetingId),
      commands.meetingsMinutesFile(meetingId),
    ]);
    if (!alive.current || mine !== seq.current) return;
    setData({
      choice: choice.status === "ok" ? choice.data : null,
      templates: templates.status === "ok" ? (templates.data ?? []) : [],
      auto: auto.status === "ok" ? (auto.data ?? null) : null,
      meta: meta.status === "ok" ? (meta.data ?? null) : null,
      file: file.status === "ok" ? file.data : null,
    });
  }, [meetingId]);

  useEffect(() => {
    alive.current = true;
    setData(EMPTY);
    void reload().catch(() => {});
    return () => {
      alive.current = false;
    };
  }, [reload]);

  // Die Wahl nach Inhalt und der Ablagepfad ändern sich beim Erzeugen.
  useEffect(() => {
    const done = (event: { payload: { meeting_id: string; kind: string } }) => {
      if (
        event.payload.meeting_id === meetingId &&
        event.payload.kind === "done"
      )
        void reload().catch(() => {});
    };
    const unNotes = events.meetingNotesEvent.listen(done);
    const unMinutes = events.minutesEvent.listen(done);
    return () => {
      void unNotes.then((f) => f());
      void unMinutes.then((f) => f());
    };
  }, [meetingId, reload]);

  const isAuto = data.choice === AUTO_TEMPLATE_ID;
  const chosenTitle = isAuto
    ? data.auto
      ? t("meetings.templates.autoShort", { title: data.auto.title })
      : t("meetings.templates.auto")
    : (data.templates.find((x) => x.id === (data.choice ?? DEFAULT_TEMPLATE_ID))
        ?.title ?? null);

  // Chip: nur, wenn es eine ausdrückliche Wahl oder ein Protokoll gibt.
  const templateName =
    data.choice !== null ? chosenTitle : (data.meta?.template_title ?? null);

  return {
    templateName,
    /** Titel der Vorlage, mit der das Protokoll erzeugt wurde (sonst `null`). */
    minutesTemplate: data.meta?.template_title ?? null,
    minutesAuto: data.meta?.auto?.outcome === "model",
    minutesFile: data.file,
    chosenTitle,
    isAuto,
    reload,
  };
}
