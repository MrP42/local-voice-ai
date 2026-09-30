import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { toast } from "sonner";
import { commands, events, type Meeting } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../ui/Button";
import { Dialog } from "../../ui/Dialog";
import { translateMeetingError } from "./meetingErrors";
import { useSelectedProject } from "./projects/selectedProject";

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

/** Wie oft und wie schnell nach der neuen Besprechung gesucht wird. */
const CLAIM_POLL_MS = 500;
const LIST_PAGE = 50;

const idsOfRecent = async (): Promise<Set<string> | null> => {
  const result = await commands.meetingsList(0, 200);
  return result.status === "ok" ? new Set(result.data.map((m) => m.id)) : null;
};

/**
 * Importiert eine Datei und haengt die neue Besprechung an `projectId`.
 *
 * Der Befehl kehrt erst am Ende der Verarbeitung zurueck (bei einer langen
 * Aufnahme Stunden spaeter), die Besprechung entsteht aber sofort. Damit sie
 * gleich im richtigen Projekt steht und ihr Fortschritt dort zu sehen ist,
 * wird sie waehrend des Laufs gesucht: die Zeile mit `source_path` = Datei, die
 * es vor dem Start noch nicht gab (Menge der bekannten IDs). Nie "die erste
 * Verarbeitung, die ich sehe": gleichzeitig kann eine Aufnahme enden oder eine
 * Neu-Transkription laufen. Gelingt der Schnappschuss der bekannten IDs nicht,
 * wird erst mit der Rueckgabe (exakte ID) zugeordnet. `error` ist der Rohtext
 * des Backends; uebersetzt wird beim Anzeigen.
 */
export async function importIntoProject(
  path: string,
  projectId: string | null,
  onCreated: (meeting: Meeting) => void,
): Promise<{ error: string | null }> {
  const known = await idsOfRecent();
  const assign = async (meeting: Meeting) => {
    if (projectId) {
      await commands.meetingsSetFolders(meeting.id, [projectId]);
    }
    notifyMeetingsChanged();
    onCreated(meeting);
  };

  let claimed = false;
  let searching = false;
  const claim = async () => {
    if (claimed || searching || known === null) return;
    searching = true;
    try {
      const result = await commands.meetingsList(0, LIST_PAGE);
      if (result.status !== "ok" || claimed) return;
      const hit = result.data.find(
        (m) => m.source_path === path && !known.has(m.id),
      );
      if (hit) {
        claimed = true;
        await assign(hit);
      }
    } finally {
      searching = false;
    }
  };

  const listening = events.meetingEvent.listen((e) => {
    if (e.payload.kind === "state" || e.payload.kind === "progress") {
      void claim();
    }
  });
  const timer = window.setInterval(() => void claim(), CLAIM_POLL_MS);
  try {
    const result = await commands.meetingsImportFile(path, true);
    if (result.status === "error") {
      return { error: result.error };
    }
    if (!claimed) {
      // Untertitel (sofort fertig) oder Suche ohne Treffer: die Rueckgabe ist die ID.
      const list = await commands.meetingsList(0, 200);
      const hit =
        list.status === "ok"
          ? list.data.find((m) => m.id === result.data)
          : undefined;
      // Wieder pruefen: die Suche waehrend des Laufs kann inzwischen gewonnen haben.
      if (!claimed) {
        if (hit) {
          claimed = true;
          await assign(hit);
        } else {
          notifyMeetingsChanged();
        }
      }
    }
    return { error: null };
  } finally {
    window.clearInterval(timer);
    void listening.then((un) => un());
  }
}

interface ImportRequest {
  paths: string[];
  projectId: string | null;
  projectName: string | null;
}

/**
 * EIN Weg fuer den Import: das Symbol "Datei importieren" und die Ablage einer
 * Datei auf der Arbeitsflaeche fuehren beide hierher. Erst die Einwilligung
 * (dass die Datei existiert, belegt keine), dann der Import in das links
 * gewaehlte Projekt. `onCreated` meldet die neue Besprechung, sobald sie
 * existiert (Auswahl, Fortschritt).
 */
export function useMeetingImport(onCreated: (meeting: Meeting) => void) {
  const { t } = useTranslation();
  const { projectId } = useSelectedProject();
  const projectRef = useRef(projectId);
  projectRef.current = projectId;
  const createdRef = useRef(onCreated);
  createdRef.current = onCreated;
  const [request, setRequest] = useState<ImportRequest | null>(null);
  const [busy, setBusy] = useState(false);

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
    setRequest({ paths, projectId: id, projectName: name });
  }, []);

  const pick = useCallback(async () => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Media", extensions: IMPORT_EXTENSIONS }],
    });
    if (typeof picked === "string") await ask([picked]);
  }, [ask]);

  const confirm = async () => {
    const current = request;
    if (!current || current.paths.length === 0) return;
    setRequest(null);
    setBusy(true);
    try {
      for (const path of current.paths) {
        const { error } = await importIntoProject(
          path,
          current.projectId,
          (meeting) => createdRef.current(meeting),
        );
        if (error) toast.error(translateMeetingError(error, t));
        notifyMeetingsChanged();
      }
    } finally {
      setBusy(false);
    }
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
            onClick={() => void confirm()}
            disabled={busy}
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
            <ul className="mt-2 space-y-0.5 text-xs text-text/60">
              {request.paths.map((p) => (
                <li key={p} className="truncate">
                  {baseName(p)}
                </li>
              ))}
            </ul>
            <p className="mt-2 text-sm font-medium" data-testid="import-target">
              {request.projectName
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
