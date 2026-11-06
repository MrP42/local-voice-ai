import React, { useRef } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { extensionOf, fileNameOf, kindOfFile } from "@/lib/library/books";
import { groupAudioFiles, type AudioGroup } from "@/lib/library/audioGroups";
import { addAudioGroup, drmMessage, findWork } from "@/lib/library/importFile";
import { useFileDrop } from "./useFileDrop";
import "./library.css";

const toLibrary = () =>
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: "library" } }),
  );

/**
 * Drag & Drop im Vorlesen (lva#152): Bücher und Dokumente öffnen wie über
 * „Hinzufügen“ (eigene Seite, wenn schon Text da ist); Hörbücher kommen in
 * die Bibliothek – an ein vorhandenes Werk ohne Audio angehängt, sonst als
 * neues Buch – und werden dort transkribiert.
 */
export const ReaderDrop: React.FC<{
  onDocument(path: string): Promise<void>;
}> = ({ onDocument }) => {
  const { t } = useTranslation();
  const queue = useRef(Promise.resolve());

  const take = async (path: string) => {
    try {
      await onDocument(path);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  /** Eine Gruppe Audiodateien (z. B. eine CD-Box) als ein Hörbuch aufnehmen. */
  const takeAudio = async (group: AudioGroup) => {
    try {
      const work = await findWork(group.title, group.authors);
      const { book } = await addAudioGroup(group, {
        // Ohne Rückfrage nur anhängen, wenn Titel UND Autor passen
        // („Track 01“ allein darf an kein fremdes Buch geraten).
        attachTo:
          work && group.authors.length > 0 && !work.audio ? work : null,
        onWarning: (m) => toast.error(m),
      });
      toast(t("library.drop.readerAudio", { title: book.title }), {
        action: { label: t("library.drop.toLibrary"), onClick: toLibrary },
      });
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    }
  };

  const drop = useFileDrop(".tts-workspace", (paths) => {
    const audio: string[] = [];
    for (const path of paths) {
      const kind = kindOfFile(path);
      if (kind === "text")
        queue.current = queue.current.then(() => take(path)).catch(() => {});
      else if (kind === "audio") audio.push(path);
      else if (kind === "drm") toast.error(drmMessage(fileNameOf(path)));
      else toast.error(t("library.unsupported", { ext: extensionOf(path) }));
    }
    for (const group of groupAudioFiles(audio))
      queue.current = queue.current.then(() => takeAudio(group)).catch(() => {});
  });

  if (!drop.over) return null;
  // Rahmen genau um die Arbeitsfläche, auf der das Ablegen zählt.
  const rect = document.querySelector(".tts-workspace")?.getBoundingClientRect();
  return (
    <div
      className="reader-drop"
      role="status"
      data-testid="reader-drop-hint"
      style={
        rect
          ? { top: rect.top, left: rect.left, width: rect.width, height: rect.height }
          : { inset: 12 }
      }
    >
      <span>{t("library.drop.readerOn")}</span>
    </div>
  );
};
