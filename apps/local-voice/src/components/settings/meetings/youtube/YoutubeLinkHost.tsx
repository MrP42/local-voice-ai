import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type Folder, type Meeting } from "@/bindings";
import { notifyMeetingsChanged } from "@/lib/meetingsBus";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { Input } from "../../../ui/Input";
import { Select } from "../../../ui/Select";
import { getSelectedProject } from "../projects/selectedProject";
import { ALL_PROJECTS, NO_PROJECT } from "../projects/projectModel";
import { looksLikeYoutubeLink, useYoutubeLinkOpen } from "./linkBus";
import { translateYoutubeError } from "./youtubeErrors";

/** Auswahlwert "Ohne Projekt" im Dialog (kein echtes Projekt hat diese ID). */
const WITHOUT_PROJECT = "__none__";

type Check =
  | { state: "empty" }
  | { state: "ok"; videoId: string; startS: number | null }
  | { state: "error"; message: string };

const formatStart = (seconds: number) => {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  const two = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
};

/** Tippt der Nutzer in ein Eingabefeld, gehoert ihm das Einfuegen. */
const isEditable = (target: EventTarget | null): boolean => {
  const el = target as HTMLElement | null;
  return Boolean(
    el &&
    (el.isContentEditable ||
      ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)),
  );
};

interface Props {
  /** Die neue Besprechung wurde angelegt (die Seite waehlt sie aus). */
  onCreated: (meeting: Meeting) => void;
  /**
   * G1 (#70): ein gewaehlter LEERER Eintrag. Dann fuellt der Link ihn (Titel
   * nur, wenn er noch der vorgeschlagene ist; Projekte und Notizen bleiben)
   * statt eine neue Besprechung anzulegen.
   */
  target?: Meeting | null;
}

/**
 * Dialog "YouTube-Link einfuegen" (#65): ein Link, ein Projekt, ein Klick. Er
 * oeffnet sich ueber das Symbol in der Aufnahmezeile oder, wenn ein YouTube-Link
 * aus der Zwischenablage eingefuegt wird (Strg+V, solange kein Eingabefeld und
 * kein Dialog den Fokus hat). Geprueft wird der Link im Backend (ohne Netz);
 * erst "Hinzufuegen" fragt einmalig Titel und Kanal bei YouTube ab.
 */
export const YoutubeLinkHost: React.FC<Props> = ({
  onCreated,
  target = null,
}) => {
  const { t } = useTranslation();
  // Das Ziel gilt so, wie es beim Oeffnen des Dialogs gewaehlt war.
  const [entry, setEntry] = useState<{ id: string; title: string } | null>(
    null,
  );
  const targetRef = useRef(target);
  targetRef.current = target;
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [check, setCheck] = useState<Check>({ state: "empty" });
  const [folders, setFolders] = useState<Folder[]>([]);
  const [projectChoice, setProjectChoice] = useState(WITHOUT_PROJECT);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const openRef = useRef(false);
  openRef.current = open;

  const openDialog = useCallback((initial: string) => {
    const chosen = targetRef.current;
    setEntry(chosen ? { id: chosen.id, title: chosen.title } : null);
    setUrl(initial);
    setError(null);
    setBusy(false);
    setOpen(true);
    // Projekt vorbelegen: das links gewaehlte, sofern es ein echtes gibt.
    void commands.meetingFoldersList().then((result) => {
      if (result.status !== "ok") return;
      const list = result.data ?? [];
      setFolders(list);
      const selected = getSelectedProject();
      const known =
        selected !== ALL_PROJECTS &&
        selected !== NO_PROJECT &&
        list.some((f) => f.id === selected);
      setProjectChoice(known ? selected : WITHOUT_PROJECT);
    });
  }, []);

  useYoutubeLinkOpen((request) => openDialog(request.url ?? ""));

  // Strg+V: ein eingefuegter YouTube-Link oeffnet den Dialog.
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      if (openRef.current || isEditable(event.target)) return;
      if (document.querySelector('[role="dialog"][aria-modal="true"]')) return;
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (!looksLikeYoutubeLink(text)) return;
      event.preventDefault();
      openDialog(text.trim());
    };
    document.addEventListener("paste", onPaste);
    return () => document.removeEventListener("paste", onPaste);
  }, [openDialog]);

  // Live pruefen (kein Netz): erkannt, oder warum nicht.
  useEffect(() => {
    if (!open) return;
    if (url.trim() === "") {
      setCheck({ state: "empty" });
      return;
    }
    let cancelled = false;
    void commands.youtubeNormalizeLink(url).then((result) => {
      if (cancelled) return;
      if (result.status === "ok") {
        setCheck({
          state: "ok",
          videoId: result.data.video_id,
          startS: result.data.start_s,
        });
      } else {
        setCheck({
          state: "error",
          message: translateYoutubeError(result.error, t),
        });
      }
    });
    return () => {
      cancelled = true;
    };
  }, [url, open, t]);

  const close = () => {
    if (!busy) setOpen(false);
  };

  const submit = async () => {
    if (busy || check.state !== "ok") return;
    setBusy(true);
    setError(null);
    const project = folders.some((f) => f.id === projectChoice)
      ? projectChoice
      : null;
    const result = await commands.youtubeAddSource(
      url,
      entry ? null : project,
      entry?.id ?? null,
    );
    setBusy(false);
    if (result.status === "error") {
      setError(translateYoutubeError(result.error, t));
      return;
    }
    setOpen(false);
    notifyMeetingsChanged();
    onCreated(result.data);
    toast.success(
      t("meetings.youtube.link.added", { title: result.data.title }),
    );
  };

  const projectOptions = [
    { value: WITHOUT_PROJECT, label: t("meetings.youtube.link.projectNone") },
    ...folders.map((f) => ({ value: f.id, label: f.name })),
  ];

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => (next ? setOpen(true) : close())}
      dismissible={!busy}
      title={t("meetings.youtube.link.title")}
      closeLabel={t("meetings.youtube.link.cancel")}
      footer={
        <>
          <Button variant="secondary" onClick={close} disabled={busy}>
            {t("meetings.youtube.link.cancel")}
          </Button>
          <Button
            onClick={() => void submit()}
            disabled={busy || check.state !== "ok"}
            data-testid="yt-add"
          >
            {busy
              ? t("meetings.youtube.link.adding")
              : t("meetings.youtube.link.add")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="yt-link-dialog">
        <div className="space-y-1">
          <label
            htmlFor="yt-link-url"
            className="text-xs font-medium text-text/60"
          >
            {t("meetings.youtube.link.urlLabel")}
          </label>
          <Input
            id="yt-link-url"
            type="text"
            value={url}
            autoFocus
            disabled={busy}
            onChange={(e) => setUrl(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void submit();
              }
            }}
            placeholder={t("meetings.youtube.link.urlPlaceholder")}
            className="w-full"
            spellCheck={false}
            autoComplete="off"
            data-testid="yt-link-input"
          />
          <p
            className="min-h-[1.25rem] text-xs"
            data-testid="yt-link-check"
            data-state={check.state}
            role="status"
          >
            {check.state === "ok" && (
              <span className="text-green-500">
                {t("meetings.youtube.link.recognized")}
                {check.startS
                  ? ` · ${t("meetings.youtube.link.startsAt", {
                      time: formatStart(check.startS),
                    })}`
                  : ""}
              </span>
            )}
            {check.state === "error" && (
              <span className="text-amber-500">{check.message}</span>
            )}
          </p>
        </div>

        {entry ? (
          <p className="text-xs text-text/70" data-testid="yt-link-target">
            {t("meetings.empty.fillsEntry", { title: entry.title })}
          </p>
        ) : (
          <div className="space-y-1" data-testid="yt-link-project">
            <span className="text-xs font-medium text-text/60">
              {t("meetings.youtube.link.projectLabel")}
            </span>
            <Select
              value={
                folders.some((f) => f.id === projectChoice)
                  ? projectChoice
                  : WITHOUT_PROJECT
              }
              options={projectOptions}
              isClearable={false}
              disabled={busy}
              menuPortal
              placeholder={t("meetings.youtube.link.projectLabel")}
              onChange={(id) => setProjectChoice(id ?? WITHOUT_PROJECT)}
            />
          </div>
        )}

        <p className="text-xs text-text/60">
          {t("meetings.youtube.link.notice")}
        </p>

        {error && (
          <p
            className="text-sm text-red-400"
            role="alert"
            data-testid="yt-link-error"
          >
            {error}
          </p>
        )}
      </div>
    </Dialog>
  );
};
