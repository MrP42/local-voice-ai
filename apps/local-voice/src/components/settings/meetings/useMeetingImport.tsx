import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { toast } from "sonner";
import { commands, type Meeting } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { translateMeetingError } from "./meetingErrors";
import { useSelectedProject } from "./projects/selectedProject";
import { findMeeting } from "./findMeeting";
import { isVideoPath } from "@/lib/meetingSlides";
import { queueSlideDetection } from "./slides/useSlidesBackground";
import { PartOrderList } from "./parts/PartOrderList";

const isSubtitlePath = (path: string) => {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return ext === "vtt" || ext === "srt";
};

/** Dieselben Endungen fuer Auswahl und Ablage (gleiche Import-Pipeline). */
export const IMPORT_EXTENSIONS = [
  "wav",
  "mp3",
  "m4a",
  "mp4",
  "mkv",
  "mov",
  "flac",
  "ogg",
  "vtt",
  "srt",
];

const hasImportExtension = (path: string) => {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMPORT_EXTENSIONS.includes(ext);
};

const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path;

/** Mehrere Audio-/Videodateien (keine Untertitel) lassen sich zu einer Besprechung zusammenfassen. */
const canCombine = (paths: string[]) =>
  paths.length >= 2 && paths.every((p) => !isSubtitlePath(p));

/** So viele Besprechungen sieht die Suche nach der neuen (sie steht ganz vorn). */
const LIST_PAGE = 50;

/**
 * Stellt eine Datei in die Import-Warteschlange und haengt die neue Besprechung
 * an `projectId`.
 *
 * Der Befehl kehrt sofort zurueck: die Besprechung gibt es schon (Status
 * "wartet", oder "laeuft", wenn sonst nichts dran ist), ihre ID ist die
 * Rueckgabe. Die Verarbeitung laeuft im Backend in der Reihenfolge des
 * Hinzufuegens; Platz und Fortschritt kommen ueber Ereignisse. `error` ist der
 * Rohtext des Backends; uebersetzt wird beim Anzeigen.
 *
 * G1 (#70): mit `targetId` fuellt die Datei einen vorhandenen LEEREN Eintrag
 * (Titel, Projekte und Notizen bleiben dort) statt eine neue Besprechung
 * anzulegen; `projectId` entfaellt dann.
 */
export async function importIntoProject(
  path: string,
  projectId: string | null,
  onCreated: (meeting: Meeting) => void,
  targetId: string | null = null,
): Promise<{ error: string | null; id: string | null }> {
  const result = await commands.meetingsImportFile(path, true, targetId);
  if (result.status === "error") {
    return { error: result.error, id: null };
  }
  const id = result.data;
  if (projectId && targetId === null) {
    await commands.meetingsSetFolders(id, [projectId]);
  }
  notifyMeetingsChanged();
  // Ein Ziel kann weit hinten in der Liste stehen: gezielt suchen.
  let meeting: Meeting | null | undefined;
  if (targetId !== null) {
    meeting = await findMeeting(id);
  } else {
    const list = await commands.meetingsList(0, LIST_PAGE);
    meeting =
      list.status === "ok" ? list.data.find((m) => m.id === id) : undefined;
  }
  if (meeting) onCreated(meeting);
  return { error: null, id };
}

interface ImportRequest {
  paths: string[];
  projectId: string | null;
  projectName: string | null;
  /** G1: leerer Eintrag, den die erste Datei fuellt. */
  target: { id: string; title: string } | null;
}

/**
 * EIN Weg fuer den Import: das Symbol "Datei importieren" und die Ablage einer
 * Datei auf der Arbeitsflaeche fuehren beide hierher. Erst die Einwilligung
 * (dass die Datei existiert, belegt keine), dann der Import in das links
 * gewaehlte Projekt. `onCreated` meldet die neue Besprechung, sobald sie
 * existiert (Auswahl, Fortschritt). U7: nichts sperrt weitere Importe; jede
 * Datei kommt in die Warteschlange und der Dialog steht sofort wieder offen.
 */
export function useMeetingImport(
  onCreated: (meeting: Meeting) => void,
  /** G1 (#70): ein gewaehlter LEERER Eintrag; die erste Datei fuellt ihn. */
  target: Meeting | null = null,
) {
  const { t } = useTranslation();
  const { projectId } = useSelectedProject();
  const projectRef = useRef(projectId);
  projectRef.current = projectId;
  const targetRef = useRef(target);
  targetRef.current = target;
  const createdRef = useRef(onCreated);
  createdRef.current = onCreated;
  const [request, setRequest] = useState<ImportRequest | null>(null);
  const [busy, setBusy] = useState(false);
  // D4: "Folien erkennen" beim Import von Videos (nur gefragt, wenn eines dabei ist).
  const [detectSlides, setDetectSlides] = useState(false);
  // Mehrere Audiodateien als EINE Besprechung (z. B. Teile vor und nach einer Pause).
  const [combine, setCombine] = useState(false);

  /** Einwilligung einholen; das Projekt gilt so, wie es jetzt links gewaehlt ist. */
  const ask = useCallback(async (paths: string[]) => {
    let id = projectRef.current;
    let name: string | null = null;
    if (id) {
      const folders = await commands.meetingFoldersList();
      const folder =
        folders.status === "ok"
          ? folders.data.find((f) => f.id === id)
          : undefined;
      // Ein inzwischen geloeschtes Projekt: ohne Projekt importieren.
      if (folder) name = folder.name;
      else id = null;
    }
    const entry = targetRef.current;
    setDetectSlides(false);
    setCombine(false);
    setRequest({
      paths,
      projectId: id,
      projectName: name,
      target: entry ? { id: entry.id, title: entry.title } : null,
    });
  }, []);

  const pick = useCallback(async () => {
    // Mehrere Dateien auf einmal: sie laufen in der gewaehlten Reihenfolge.
    const picked = await open({
      multiple: true,
      filters: [{ name: "Media", extensions: IMPORT_EXTENSIONS }],
    });
    const paths = Array.isArray(picked)
      ? picked
      : typeof picked === "string"
        ? [picked]
        : [];
    if (paths.length > 0) await ask(paths);
  }, [ask]);

  const confirmImport = async () => {
    const current = request;
    if (!current || current.paths.length === 0) return;
    setRequest(null);
    setBusy(true);
    try {
      if (combine && canCombine(current.paths)) {
        await importCombined(current);
        return;
      }
      for (const [index, path] of current.paths.entries()) {
        const { error, id } = await importIntoProject(
          path,
          current.projectId,
          (meeting) => {
            // Mit einem Ziel bleibt dieses gewaehlt: die weiteren Dateien
            // laufen links mit, ohne die Ansicht zu wechseln.
            if (index === 0 || !current.target) createdRef.current(meeting);
          },
          // Nur die erste Datei fuellt den leeren Eintrag.
          index === 0 ? (current.target?.id ?? null) : null,
        );
        if (error) toast.error(translateMeetingError(error, t));
        // Die Folienerkennung folgt dem Import: sie startet, sobald er fertig ist.
        else if (id && detectSlides && isVideoPath(path)) {
          queueSlideDetection(id);
        }
        notifyMeetingsChanged();
      }
    } finally {
      setBusy(false);
    }
  };

  /**
   * Mehrere Dateien als eine Besprechung: die erste wird wie gewohnt importiert
   * (Projekt, leerer Eintrag), die weiteren werden an sie angehaengt. Alle
   * laufen als eigene Auftraege durch die Warteschlange; sobald der letzte Teil
   * fertig ist, fuehrt das Backend sie zu einer Zeitachse zusammen.
   */
  const importCombined = async (current: ImportRequest) => {
    const [first, ...rest] = current.paths;
    const { error, id } = await importIntoProject(
      first,
      current.projectId,
      (meeting) => createdRef.current(meeting),
      current.target?.id ?? null,
    );
    if (error || !id) {
      if (error) toast.error(translateMeetingError(error, t));
      return;
    }
    const appended = await commands.meetingsAppendFiles(id, rest, true);
    if (appended.status === "error") {
      toast.error(translateMeetingError(appended.error, t));
    } else {
      toast.success(
        t("meetings.parts.import.queued", { count: current.paths.length }),
      );
    }
    current.paths.forEach((path, index) => {
      if (index === 0 && detectSlides && isVideoPath(path)) {
        queueSlideDetection(id);
      }
    });
    notifyMeetingsChanged();
  };

  const dialog = (
    <Dialog
      open={request !== null}
      onOpenChange={(isOpen) => {
        if (!isOpen) setRequest(null);
      }}
      title={t("meetings.consent.title")}
      closeLabel={t("meetings.consent.cancel")}
      footer={
        <>
          <Button variant="secondary" onClick={() => setRequest(null)}>
            {t("meetings.consent.cancel")}
          </Button>
          <Button
            onClick={() => void confirmImport()}
            data-testid="import-confirm"
          >
            {t("meetings.consent.confirm")}
          </Button>
        </>
      }
    >
      <div data-testid="import-dialog">
        <p className="whitespace-pre-wrap text-sm text-text/80">
          {t("meetings.consent.importBody")}
        </p>
        {request && (
          <>
            {combine && canCombine(request.paths) ? (
              <div className="mt-2">
                <PartOrderList
                  items={request.paths.map((p) => ({
                    id: p,
                    label: baseName(p),
                  }))}
                  onChange={(items) =>
                    setRequest({ ...request, paths: items.map((i) => i.id) })
                  }
                  testId="import-order"
                />
              </div>
            ) : (
              <ul className="mt-2 space-y-0.5 text-xs text-text/60">
                {request.paths.map((p) => (
                  <li key={p} className="truncate">
                    {baseName(p)}
                  </li>
                ))}
              </ul>
            )}
            {canCombine(request.paths) && (
              <label className="mt-3 flex cursor-pointer items-start gap-2 text-sm max-sm:min-h-[44px]">
                <input
                  type="checkbox"
                  className="mt-0.5 max-sm:h-5 max-sm:w-5"
                  data-testid="import-combine"
                  checked={combine}
                  onChange={(e) => setCombine(e.target.checked)}
                />
                <span>
                  {t("meetings.parts.import.combine")}
                  <span className="block text-xs text-text/60">
                    {t("meetings.parts.import.combineHint")}
                  </span>
                </span>
              </label>
            )}
            {request.paths.some(isVideoPath) && (
              <label className="mt-3 flex cursor-pointer items-start gap-2 text-sm">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  data-testid="import-detect-slides"
                  checked={detectSlides}
                  onChange={(e) => setDetectSlides(e.target.checked)}
                />
                <span>
                  {t("meetings.slides.importOption")}
                  <span className="block text-xs text-text/60">
                    {t("meetings.slides.importOptionHint")}
                  </span>
                </span>
              </label>
            )}
            <p className="mt-2 text-sm font-medium" data-testid="import-target">
              {request.target
                ? t("meetings.empty.importFirst", {
                    title: request.target.title,
                  })
                : request.projectName
                  ? t("meetings.import.target", { name: request.projectName })
                  : t("meetings.import.targetNone")}
            </p>
          </>
        )}
      </div>
    </Dialog>
  );

  return { pick, ask, busy, dialog };
}

/** Liegt der Punkt (physische Pixel des Fensters) auf der Arbeitsflaeche? */
const insideDropZone = (position: { x: number; y: number }): boolean => {
  const zone = document.querySelector("[data-rec-dropzone]");
  if (!zone) return false;
  const rect = zone.getBoundingClientRect();
  const scale = window.devicePixelRatio || 1;
  const x = position.x / scale;
  const y = position.y / scale;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
};

/**
 * Datei-Ablage auf der Arbeitsflaeche (Mitte). Tauri faengt Datei-Drops unter
 * Windows fenster-weit ab (HTML5-Ziehen kommt nicht an): darum das Tauri-
 * Ereignis mit Trefferpruefung auf die Arbeitsflaeche. Ein Drop daneben tut
 * nichts. Liefert `over`, solange eine Datei ueber der Flaeche schwebt.
 */
export function useImportDrop(onPaths: (paths: string[]) => void): boolean {
  const { t } = useTranslation();
  const [over, setOver] = useState(false);
  const pathsRef = useRef(onPaths);
  pathsRef.current = onPaths;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "leave") {
          setOver(false);
          return;
        }
        const inside = insideDropZone(payload.position);
        if (payload.type !== "drop") {
          setOver(inside);
          return;
        }
        setOver(false);
        if (!inside) return;
        const accepted = payload.paths.filter(hasImportExtension);
        if (accepted.length === 0) {
          toast.error(t("meetings.errors.unsupportedFile"));
          return;
        }
        pathsRef.current(accepted);
      })
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [t]);

  return over;
}
