import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, X } from "lucide-react";
import { commands, type MeetingDocument } from "@/bindings";
import { useTextHistory } from "@/hooks/useTextHistory";
import { saveErrorCode } from "@/lib/minutesEdit";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { EditorMenu } from "../tts/editor/EditorMenu";
import { HistoryButtons } from "../tts/editor/HistoryButtons";
import {
  TtsChipEditor,
  type ChipEditorInsertApi,
} from "../tts/editor/TtsChipEditor";

const SPELL_KEY = "lv.minutesEditor.spellcheck";

const readSpell = (): boolean => {
  try {
    return localStorage.getItem(SPELL_KEY) !== "0";
  } catch {
    return true;
  }
};

interface MinutesEditorProps {
  doc: MeetingDocument;
  /** Bereich, der beim Öffnen sichtbar und markiert sein soll (angeklickter Block). */
  reveal?: { start: number; end: number } | null;
  /** Gespeichert: das Dokument, wie es jetzt in der Datenbank steht. */
  onSaved(doc: MeetingDocument): void;
  /** Bearbeiten beenden ohne zu speichern (Rückfrage bei Änderungen erledigt der Editor). */
  onClose(): void;
  /** Ob ungespeicherte Änderungen bestehen (sperrt „Neu erzeugen“ in der Werkzeugzeile). */
  onDirtyChange?(dirty: boolean): void;
}

/**
 * Bearbeiten-Ansicht eines Protokolls oder Dokumenttyps: derselbe Editor wie
 * beim Vorlesen (Editor-Menü mit Suchen und Ersetzen, Formatierung,
 * Bereinigen, KI-Textarbeit; Rückgängig/Wiederherstellen), hier auf dem
 * Markdown des Dokuments. Gespeichert wird in die bestehende Version
 * (optimistische Sperre): Export, Kopieren, Mail und Vorlesen lesen danach
 * den korrigierten Text.
 */
export const MinutesEditor: React.FC<MinutesEditorProps> = ({
  doc,
  reveal,
  onSaved,
  onClose,
  onDirtyChange,
}) => {
  const { t } = useTranslation();
  const [text, setText] = useState(doc.body);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDiscard, setConfirmDiscard] = useState(false);
  const [spellCheck, setSpellCheck] = useState(readSpell);
  const apiRef = useRef<ChipEditorInsertApi | null>(null);
  const history = useTextHistory(`minutes-edit:${doc.id}`, text, setText);
  const dirty = text !== doc.body;

  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);
  useEffect(() => () => onDirtyChange?.(false), [onDirtyChange]);

  // Angeklickte Stelle zeigen, sobald das Textfeld steht.
  useEffect(() => {
    const id = window.requestAnimationFrame(() => {
      const api = apiRef.current;
      if (reveal) api?.revealRange?.(reveal.start, reveal.end);
      else api?.revealRange?.(0, 0);
    });
    return () => window.cancelAnimationFrame(id);
    // Nur beim Öffnen: spätere Änderungen von `reveal` gelten dem nächsten Öffnen.
  }, []);

  const changeSpell = (on: boolean) => {
    setSpellCheck(on);
    try {
      localStorage.setItem(SPELL_KEY, on ? "1" : "0");
    } catch {
      // Ohne Speicher gilt die Wahl nur bis zum Schließen.
    }
  };

  const save = useCallback(async () => {
    if (saving) return;
    if (!dirty) {
      onClose();
      return;
    }
    setSaving(true);
    setError(null);
    const result = await commands.meetingsUpdateDocumentText(
      doc.id,
      text,
      doc.updated_at,
    );
    setSaving(false);
    if (result.status !== "ok") {
      const code = saveErrorCode(result.error);
      setError(
        t(`meetings.minutes.edit.errors.${code}`, { error: result.error }),
      );
      return;
    }
    onSaved(result.data);
  }, [saving, dirty, doc.id, doc.updated_at, text, onSaved, onClose, t]);

  const cancel = () => {
    if (dirty) setConfirmDiscard(true);
    else onClose();
  };

  // Strg+S speichert, Esc bricht ab (nicht in Dialogen und nicht in der Suchleiste).
  const onKeyDown = (event: React.KeyboardEvent) => {
    const mod = event.ctrlKey || event.metaKey;
    if (mod && !event.altKey && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void save();
      return;
    }
    if (event.key === "Escape" && !event.defaultPrevented) {
      const target = event.target as Element | null;
      if (target?.closest?.('[data-testid="find-bar"]')) return;
      event.preventDefault();
      cancel();
    }
  };

  return (
    <div
      className="minutes-edit flex flex-col gap-2"
      data-testid="minutes-editor"
      onKeyDown={onKeyDown}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="me-auto text-sm font-semibold">
          {t("meetings.minutes.edit.title", { version: doc.version })}
        </span>
        <HistoryButtons
          past={history.past}
          future={history.future}
          present={history.present}
          onUndo={history.undo}
          onRedo={history.redo}
          onJump={history.jump}
        />
        <Button
          size="sm"
          variant="secondary"
          onClick={cancel}
          data-testid="minutes-edit-cancel"
          disabled={saving}
          icon={<X width={14} height={14} aria-hidden="true" />}
        >
          {t("meetings.detail.cancel")}
        </Button>
        <Button
          size="sm"
          variant="primary"
          onClick={() => void save()}
          data-testid="minutes-edit-save"
          disabled={saving || text.trim().length === 0}
          title={t("meetings.minutes.edit.saveHint")}
          icon={<Check width={14} height={14} aria-hidden="true" />}
        >
          {saving
            ? t("meetings.minutes.edit.saving")
            : t("meetings.detail.save")}
        </Button>
      </div>

      {confirmDiscard && (
        <Alert variant="warning">
          <div
            className="flex flex-wrap items-center gap-2"
            data-testid="minutes-edit-discard"
          >
            <span className="me-auto">
              {t("meetings.minutes.edit.discard")}
            </span>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => setConfirmDiscard(false)}
              data-testid="minutes-edit-keep"
            >
              {t("meetings.minutes.edit.keepEditing")}
            </Button>
            <Button
              size="sm"
              variant="danger"
              onClick={onClose}
              data-testid="minutes-edit-discard-go"
            >
              {t("meetings.minutes.edit.discardGo")}
            </Button>
          </div>
        </Alert>
      )}

      {error && (
        <Alert variant="error">
          <span data-testid="minutes-edit-error">{error}</span>
        </Alert>
      )}

      <EditorMenu
        value={text}
        onChange={setText}
        readOnly={saving}
        apiRef={apiRef}
        spellCheck={spellCheck}
        onSpellCheck={changeSpell}
      />
      <TtsChipEditor
        value={text}
        onChange={setText}
        providers={[]}
        insertApiRef={apiRef}
        spellCheck={spellCheck}
        rows={22}
        placeholder={t("meetings.minutes.edit.placeholder")}
        className="w-full"
        onUndo={history.undo}
        onRedo={history.redo}
      />
      <p className="text-xs text-text/60">{t("meetings.minutes.edit.hint")}</p>
    </div>
  );
};
