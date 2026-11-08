import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pencil } from "lucide-react";
import type { VoiceLine, VoiceSource } from "@/stores/voiceCallStore";
import { sourceTarget } from "./globalContext";
import { openInReader } from "@/lib/library/books";

/** Beleg öffnen: Seite im Vorlesen oder den Bereich der Quelle. */
function openSource(src: VoiceSource): void {
  const target = sourceTarget(src.jump);
  if (!target) return;
  if ("page" in target) {
    openInReader(target.page, false);
    return;
  }
  window.dispatchEvent(
    new CustomEvent("lv-navigate", { detail: { section: target.section } }),
  );
}

/** Bearbeiten der eigenen erkannten Zeile: Enter speichert, Esc bricht ab (Umschalt+Enter = neue Zeile). */
const LineEditor: React.FC<{
  initial: string;
  onSave: (text: string) => void;
  onCancel: () => void;
  /** Meldet jede Änderung des Entwurfs, damit nichts still verloren geht. */
  onDraft: (text: string) => void;
}> = ({ initial, onSave, onCancel, onDraft }) => {
  const { t } = useTranslation();
  const [value, setValue] = useState(initial);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  return (
    <div className="flex flex-col gap-1 w-[min(520px,70vw)]">
      <textarea
        ref={ref}
        rows={Math.min(6, Math.max(2, Math.ceil(initial.length / 48)))}
        value={value}
        aria-label={t("voiceCall.edit.field")}
        aria-describedby="voice-line-edit-hint"
        data-testid="line-editor"
        className="w-full rounded-xl border border-logo-primary bg-background px-3 py-2 text-sm text-text resize-y"
        onChange={(e) => {
          setValue(e.target.value);
          onDraft(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            onCancel();
          } else if (
            e.key === "Enter" &&
            !e.shiftKey &&
            !e.nativeEvent.isComposing
          ) {
            e.preventDefault();
            onSave(value);
          }
        }}
      />
      <span id="voice-line-edit-hint" className="text-[11px] text-text/60">
        {t("voiceCall.edit.hint")}
      </span>
    </div>
  );
};

export const Transcript: React.FC<{
  lines: VoiceLine[];
  active: boolean;
  /**
   * Leeres Gespräch (lva#394, Vorbild ChatGPT): Begrüßung und Eingabezeile mittig,
   * keine Vorschläge. Ohne Angabe bleibt die Fläche leer.
   */
  greeting?: string;

  /** Werkzeug, das der Orchestrator gerade aufruft (Name). */
  toolStatus?: string | null;
  /** Hinweis des Orchestrators (Code). */
  notice?: string | null;
  /** Öffnet die Einstellungen (Reiter Modelle), z. B. aus dem Hinweis zum gesperrten Modell. */
  onOpenSettings?: () => void;
  /** Speichert die verbesserte Fassung einer eigenen erkannten Zeile. Ohne sie gibt es keinen Stift. */
  onEditLine?: (id: number, text: string) => void;
  /** Darf die Zeile jetzt bearbeitet werden? Ohne Angabe gilt jede gesprochene Nutzerzeile. */
  canEdit?: (line: VoiceLine) => boolean;
}> = ({
  lines,
  active,
  greeting,
  toolStatus = null,
  notice = null,
  onOpenSettings,
  onEditLine,
  canEdit,
}) => {
  const { t, i18n } = useTranslation();
  const endRef = useRef<HTMLDivElement>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const draft = useRef("");
  /** Entwurf einer Zeile, die während der Bearbeitung ersetzt oder gesperrt wurde. */
  const [lost, setLost] = useState<string | null>(null);
  const pencils = useRef(new Map<number, HTMLButtonElement>());
  /** Nach dem Schließen des Editors geht der Fokus zurück zum Stift dieser Zeile. */
  const [refocus, setRefocus] = useState<number | null>(null);
  const isEditable = (line: VoiceLine) =>
    line.role === "user" &&
    line.mode === "spoken" &&
    !!onEditLine &&
    (canEdit ? canEdit(line) : true);

  // Verschwindet die Zeile oder wird sie gesperrt (neue Äußerung, Antwort läuft), schließt der
  // Editor; der Entwurf bleibt in einem Hinweis stehen.
  useEffect(() => {
    if (editing === null) return;
    const line = lines.find((l) => l.id === editing);
    if (line && isEditable(line)) return;
    setEditing(null);
    if (draft.current.trim() && draft.current !== line?.text)
      setLost(draft.current);
  });

  useEffect(() => {
    if (refocus === null || editing !== null) return;
    pencils.current.get(refocus)?.focus();
    setRefocus(null);
  }, [refocus, editing]);

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "end" });
  }, [lines, toolStatus, notice]);

  // Die Live-Region ist immer da (leer), damit Ansagen zuverlässig ankommen; sie trägt nur die
  // Werkzeugzeile. Der Hinweis ist ein eigenes Element, ein gesperrtes Modell meldet sich als Alarm.
  const liveRegion = (
    <div
      role="status"
      aria-live="polite"
      data-testid="call-status"
      className="self-start text-xs text-text/60 italic min-h-[1em]"
    >
      {toolStatus
        ? t([`voiceCall.tool.${toolStatus}`, "voiceCall.tool.generic"], {
            name: toolStatus,
          })
        : null}
    </div>
  );
  const noticeBox = notice ? (
    <div
      role={notice === "speaker_blocked" ? "alert" : "note"}
      data-testid="call-notice"
      className="self-start max-w-full rounded-lg border border-mid-gray/30 bg-mid-gray/10 px-3 py-2 text-sm flex flex-wrap items-center gap-2"
    >
      <span>{t(`voiceCall.notice.${notice}`)}</span>
      {onOpenSettings && (
        <button
          type="button"
          onClick={onOpenSettings}
          className="rounded border border-mid-gray/40 bg-background px-2 py-0.5 text-xs cursor-pointer text-text hover:border-logo-primary"
        >
          {t("voiceCall.notice.openSettings")}
        </button>
      )}
    </div>
  ) : null;

  if (lines.length === 0) {
    // Die Eingabezeile steht unmittelbar darunter (VoiceCallPage, feste Baumposition):
    // die Begrüßung sitzt unten in dieser Fläche, beide zusammen in der Mitte.
    const title = active ? t("voiceCall.speakNow") : greeting;
    return (
      <div className="flex-1 min-h-0 flex flex-col items-center justify-end gap-3 px-4 pb-6 text-center">
        {title && (
          <h2
            className="m-0 text-[28px] max-sm:text-[22px] font-semibold text-text tracking-tight"
            data-testid="chat-greeting"
          >
            {title}
          </h2>
        )}
        {noticeBox}
        {liveRegion}
      </div>
    );
  }

  const time = (at: number) =>
    new Date(at).toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
    });

  return (
    <div
      className="flex-1 min-h-0 overflow-y-auto px-4 pt-4 pb-2"
      role="log"
      aria-label={t("voiceCall.transcript")}
    >
      <div className="mx-auto max-w-[880px] flex flex-col gap-2.5">
        {lines.map((line) => {
          const user = line.role === "user";
          const editable = isEditable(line);
          const isEditing = editing === line.id && editable;
          return (
            <div
              key={line.id}
              className={`flex gap-2 items-end max-w-[85%] ${user ? "self-end flex-row-reverse" : "self-start"}`}
            >
              {!user && (
                <span
                  className="grid place-items-center w-7 h-7 rounded-full shrink-0 text-[11px] font-semibold bg-logo-primary/15 text-logo-primary"
                  aria-hidden="true"
                >
                  {t("voiceCall.aiAvatar")}
                </span>
              )}
              <div className="min-w-0">
                {isEditing ? (
                  <LineEditor
                    initial={line.text}
                    onDraft={(text) => (draft.current = text)}
                    onCancel={() => {
                      setEditing(null);
                      setRefocus(line.id);
                    }}
                    onSave={(text) => {
                      setEditing(null);
                      setRefocus(line.id);
                      onEditLine?.(line.id, text);
                    }}
                  />
                ) : (
                  <div
                    className={`px-3 py-2 rounded-xl text-sm whitespace-pre-wrap break-words ${
                      user
                        ? "bg-logo-primary/15 rounded-br-sm"
                        : "bg-mid-gray/15 rounded-bl-sm"
                    }`}
                  >
                    {line.text}
                    {line.interrupted ? " …" : ""}
                    {line.sources && line.sources.length > 0 && (
                      <ul
                        aria-label={t("voiceCall.sources.label")}
                        data-testid="voice-sources"
                        className="m-0 mt-1.5 p-0 list-none flex flex-col gap-0.5 text-xs text-text/70"
                      >
                        {line.sources.map((src) => (
                          <li key={src.k}>
                            <button
                              type="button"
                              data-testid="voice-source"
                              disabled={!sourceTarget(src.jump)}
                              onClick={() => openSource(src)}
                              title={t("voiceCall.sources.open")}
                              className="text-left underline-offset-2 hover:underline disabled:no-underline disabled:cursor-default"
                            >
                              {t("voiceCall.sources.entry", {
                                k: src.k,
                                kind: t(`voiceCall.sources.kind.${src.kind}`, {
                                  defaultValue: src.kind,
                                }),
                                title: src.title,
                              })}
                              {src.page !== null
                                ? `, ${t("voiceCall.sources.page", { page: src.page })}`
                                : ""}
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                )}
                <div
                  className={`mt-0.5 flex gap-2 text-[11px] text-text/50 ${user ? "justify-end" : ""}`}
                >
                  {editable && !isEditing && (
                    <button
                      type="button"
                      aria-label={t("voiceCall.edit.button")}
                      title={t("voiceCall.edit.button")}
                      ref={(el) => {
                        if (el) pencils.current.set(line.id, el);
                        else pencils.current.delete(line.id);
                      }}
                      onClick={() => {
                        draft.current = line.text;
                        setLost(null);
                        setEditing(line.id);
                      }}
                      className="grid place-items-center w-7 h-7 -my-1.5 rounded cursor-pointer text-text/80 hover:bg-mid-gray/15 hover:text-logo-primary focus-visible:text-text"
                    >
                      <Pencil className="w-4 h-4" aria-hidden="true" />
                    </button>
                  )}
                  <span>{time(line.at)}</span>
                  {line.latencyMs !== undefined && (
                    <span>
                      {t("voiceCall.seconds", {
                        value: (line.latencyMs / 1000).toLocaleString(
                          i18n.language,
                          { maximumFractionDigits: 1 },
                        ),
                      })}
                    </span>
                  )}
                </div>
              </div>
            </div>
          );
        })}
        {lost !== null && (
          <div
            role="alert"
            data-testid="edit-lost"
            className="self-end max-w-[85%] rounded-lg border border-mid-gray/30 bg-mid-gray/10 px-3 py-2 text-sm flex flex-col gap-1"
          >
            <span>{t("voiceCall.edit.lost")}</span>
            <span className="italic break-words">{lost}</span>
            <button
              type="button"
              onClick={() => setLost(null)}
              className="self-start rounded border border-mid-gray/40 bg-background px-2 py-0.5 text-xs cursor-pointer text-text hover:border-logo-primary"
            >
              {t("voiceCall.edit.dismiss")}
            </button>
          </div>
        )}
        {noticeBox}
        {liveRegion}
        <div ref={endRef} />
      </div>
    </div>
  );
};
