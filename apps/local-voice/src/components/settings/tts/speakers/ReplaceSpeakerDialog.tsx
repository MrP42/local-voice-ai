import React, { useEffect, useMemo, useRef, useState } from "react";
import { Select as FluentSelect } from "@fluentui/react-components";
import { useTranslation } from "react-i18next";
import { commands, type PageInfo } from "@/bindings";
import { Button } from "../../../ui/Button";
import { Dialog } from "../../../ui/Dialog";
import { foldSpeakerName, type SpeakerRef } from "@/lib/voices/speakerMarkers";
import {
  fieldsOfState,
  replaceSpeakerInFields,
  type SpeakerFields,
} from "@/lib/voices/replaceSpeaker";
import { providerVoiceValue } from "../providers/providerModel";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  speakers: SpeakerRef[];
  /** Anbieter, dessen Sprecher gelten (`null` = Fish): bestimmt den Dropdown-Wert einer Stimme. */
  providerId: string | null;
  pages: PageInfo[];
  activePageId: string;
  /** Der offene Stand der aktiven Seite (noch nicht unbedingt gespeichert). */
  active: SpeakerFields;
  onApplyActive: (next: SpeakerFields) => void;
  /** Nach dem Speichern anderer Seiten (Seitenliste neu laden). */
  onSaved?: () => void;
  /** Vorbelegung "Ersetze": ein Marker-Name oder eine Sprecher-id. */
  initialFrom?: string;
}

const valueOf = (providerId: string | null, speaker: SpeakerRef) =>
  providerId ? providerVoiceValue(providerId, speaker.id) : speaker.id;

type Loaded = {
  id: string;
  state: Record<string, unknown>;
  fields: SpeakerFields;
};

/**
 * Stimme X auf allen Seiten durch Y ersetzen: die Marker `<X>` in Text und
 * Zusammenfassung jeder Seite und die dort gemerkte Reiter-Stimme. Die Vorschau
 * zaehlt vorab, was sich aendert. Uebersetzungen im Zwischenspeicher bleiben
 * unberuehrt (sie entstehen aus dem Original neu).
 */
export const ReplaceSpeakerDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  speakers,
  providerId,
  pages,
  activePageId,
  active,
  onApplyActive,
  onSaved,
  initialFrom,
}) => {
  const { t } = useTranslation();
  const [fromId, setFromId] = useState("");
  const [toId, setToId] = useState("");
  const [others, setOthers] = useState<Loaded[]>([]);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [done, setDone] = useState<{
    markers: number;
    pages: number;
    failed: number;
  } | null>(null);
  const speakersRef = useRef(speakers);
  speakersRef.current = speakers;

  const defaults = (list: SpeakerRef[], from?: string) => {
    const needle = from ? foldSpeakerName(from) : "";
    const match =
      list.find(
        (s) =>
          foldSpeakerName(s.id) === needle ||
          foldSpeakerName(s.displayName) === needle ||
          (s.aliases ?? []).some((a) => foldSpeakerName(a) === needle),
      ) ?? list[0];
    setFromId(match?.id ?? "");
    setToId(list.find((s) => s.id !== match?.id)?.id ?? "");
  };
  // Beim Oeffnen vorbelegen -- nicht bei jeder neuen Sprecherliste (Stimmen-
  // ereignisse), das wuerde die Wahl und das "Fertig" zuruecksetzen.
  useEffect(() => {
    if (!open) return;
    setDone(null);
    defaults(speakersRef.current, initialFrom);
  }, [open, initialFrom]);
  // Verschwindet eine gewaehlte Stimme aus der Liste (oder kommt sie erst jetzt),
  // neu vorbelegen.
  useEffect(() => {
    if (!open) return;
    if (
      !speakers.some((s) => s.id === fromId) ||
      !speakers.some((s) => s.id === toId)
    ) {
      defaults(speakers, initialFrom);
    }
  }, [speakers]);

  useEffect(() => {
    if (!open) return;
    let alive = true;
    setOthers([]);
    setLoaded(false);
    void Promise.all(
      pages
        .filter((p) => p.id !== activePageId)
        .map(async (p): Promise<Loaded | null> => {
          const result = await commands.pageStateLoad(p.id).catch(() => null);
          const parsed = fieldsOfState(
            result && result.status === "ok" ? result.data : null,
          );
          return parsed ? { id: p.id, ...parsed } : null;
        }),
    ).then((loaded) => {
      if (!alive) return;
      setOthers(loaded.filter((l): l is Loaded => l !== null));
      setLoaded(true);
    });
    return () => {
      alive = false;
    };
  }, [open, pages, activePageId]);

  const from = speakers.find((s) => s.id === fromId);
  const to = speakers.find((s) => s.id === toId);

  const plan = useMemo(() => {
    if (!from || !to || from.id === to.id) return null;
    const fromValue = valueOf(providerId, from);
    const toValue = valueOf(providerId, to);
    const run = (fields: SpeakerFields) =>
      replaceSpeakerInFields(fields, from, to, speakers, fromValue, toValue);
    const activeResult = run(active);
    const otherResults = others.map((o) => ({ ...o, result: run(o.fields) }));
    const changed = (r: { markers: number; voices: number }) =>
      r.markers + r.voices > 0;
    return {
      activeResult,
      otherResults: otherResults.filter((o) => changed(o.result)),
      markers:
        activeResult.markers +
        otherResults.reduce((n, o) => n + o.result.markers, 0),
      pages:
        (changed(activeResult) ? 1 : 0) +
        otherResults.filter((o) => changed(o.result)).length,
    };
  }, [from, to, speakers, providerId, active, others]);

  const apply = async () => {
    if (!from || !to || from.id === to.id) return;
    setBusy(true);
    const fromValue = valueOf(providerId, from);
    const toValue = valueOf(providerId, to);
    let markers = 0;
    let pagesChanged = 0;
    let failed = 0;
    try {
      // Jede Seite direkt vor dem Schreiben frisch laden: der Stand der Vorschau
      // kann inzwischen veraltet sein (Autosave, Sync).
      for (const page of pages.filter((p) => p.id !== activePageId)) {
        const result = await commands.pageStateLoad(page.id).catch(() => null);
        const parsed = fieldsOfState(
          result && result.status === "ok" ? result.data : null,
        );
        if (!parsed) continue;
        const r = replaceSpeakerInFields(
          parsed.fields,
          from,
          to,
          speakers,
          fromValue,
          toValue,
        );
        if (r.markers + r.voices === 0) continue;
        const saved = await commands
          .pageStateSave(
            page.id,
            JSON.stringify({
              ...parsed.state,
              text: r.fields.text,
              summary: r.fields.summary,
              voices: r.fields.voices,
            }),
          )
          .catch(() => null);
        if (saved && saved.status === "ok") {
          markers += r.markers;
          pagesChanged += 1;
        } else {
          failed += 1;
        }
      }
      const current = replaceSpeakerInFields(
        active,
        from,
        to,
        speakers,
        fromValue,
        toValue,
      );
      if (current.markers + current.voices > 0) {
        onApplyActive(current.fields);
        markers += current.markers;
        pagesChanged += 1;
      }
      if (pagesChanged > 0) {
        void commands.syncTouch();
        onSaved?.();
      }
    } finally {
      setBusy(false);
    }
    setDone({ markers, pages: pagesChanged, failed });
  };

  const options = speakers.map((s) => (
    <option key={s.id} value={s.id}>
      {s.displayName}
    </option>
  ));

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("tts.replaceSpeaker.title")}
      description={t("tts.replaceSpeaker.description")}
      closeLabel={t("tts.replaceSpeaker.close")}
      className="max-w-lg"
      footer={
        <div className="flex justify-end gap-2">
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {done
              ? t("tts.replaceSpeaker.close")
              : t("tts.replaceSpeaker.cancel")}
          </Button>
          {!done && (
            <Button
              onClick={() => void apply()}
              disabled={
                busy || !loaded || !plan || plan.markers + plan.pages === 0
              }
              data-testid="replace-speaker-apply"
            >
              {t("tts.replaceSpeaker.apply")}
            </Button>
          )}
        </div>
      }
    >
      <div className="space-y-3 text-sm">
        <label className="flex items-center gap-3">
          <span className="w-20 shrink-0">{t("tts.replaceSpeaker.from")}</span>
          <FluentSelect
            value={fromId}
            onChange={(e) => setFromId(e.target.value)}
            className="flex-1"
            data-testid="replace-speaker-from"
          >
            {options}
          </FluentSelect>
        </label>
        <label className="flex items-center gap-3">
          <span className="w-20 shrink-0">{t("tts.replaceSpeaker.to")}</span>
          <FluentSelect
            value={toId}
            onChange={(e) => setToId(e.target.value)}
            className="flex-1"
            data-testid="replace-speaker-to"
          >
            {options}
          </FluentSelect>
        </label>
        <p
          className="text-text/70"
          role="status"
          data-testid="replace-speaker-preview"
        >
          {done
            ? done.failed > 0
              ? t("tts.replaceSpeaker.doneFailed", done)
              : t("tts.replaceSpeaker.done", done)
            : plan
              ? plan.markers + plan.pages === 0
                ? t("tts.replaceSpeaker.none")
                : t("tts.replaceSpeaker.preview", {
                    markers: plan.markers,
                    pages: plan.pages,
                  })
              : t("tts.replaceSpeaker.pick")}
        </p>
      </div>
    </Dialog>
  );
};
