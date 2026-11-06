import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type BookTalkCitation } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import { useSettings } from "../../../../hooks/useSettings";
import { tauriBookTalkPort } from "./api";
import { BookTalkColumn } from "./BookTalkColumn";
import { ConsentDialog } from "./ConsentDialog";
import { SubscriptionPreviewDialog } from "./SubscriptionPreviewDialog";
import {
  bookModelOf,
  consentKeyOf,
  pendingConsent,
  withoutConsent,
} from "./consent";
import { pageAt, scopeLimit } from "./pages";
import type { BookTalkPort } from "./port";
import { useBookTalk } from "./useBookTalk";
import { useBookVoice, type BookTalkHost } from "./useBookVoice";

/**
 * Gesprächsspalte mit allem, was sie braucht (lva#309): Verlauf und Anfragen,
 * Sprachgespräch, Cloud-Freigabe. Sitzt in der rechten Reader-Spalte und im
 * Blatt „Gespräch“ des schmalen Fensters; beim Schließen endet die Sitzung,
 * ein erweiterter Bereich („Ganzes Buch“) fällt auf „bis hier“ zurück.
 */
export function BookTalkPanel({
  bookId,
  text,
  chapterStarts = [],
  frontier,
  host,
  port = tauriBookTalkPort,
  keyboard = true,
  onNavigate,
}: {
  bookId: string;
  /** Originaltext des Buchs (für die Seitenzahl an der Lesegrenze). */
  text: string;
  /** Kapitelanfänge (UTF-16) aus dem Inhaltsverzeichnis, für „Bis Kap. k“ ohne Seitenmarken. */
  chapterStarts?: readonly number[];
  /** Lesegrenze (UTF-16); `undefined`, solange sie unbekannt ist. */
  frontier(): number | undefined;
  host: BookTalkHost;
  port?: BookTalkPort;
  keyboard?: boolean;
  /** Nach „Hinspringen“ und „Ab hier vorlesen“: das Blatt (schmales Fenster) schließt, der Text ist zu sehen. */
  onNavigate?(): void;
}) {
  const { settings, updateSetting } = useSettings();
  const [boundary, setBoundary] = useState<number | undefined>(() => frontier());
  /** Wartende Handlung hinter dem Freigabedialog (Frage stellen, Sitzung starten). */
  const [consent, setConsent] = useState<{ resume(): void } | null>(null);

  // Die Grenze wächst mit dem Lesen und Hören; der Fortschrittsspeicher meldet jede Änderung.
  useEffect(() => {
    const read = () => setBoundary(frontier());
    read();
    window.addEventListener(CHANGED_EVENT, read);
    window.addEventListener(HYDRATED_EVENT, read);
    return () => {
      window.removeEventListener(CHANGED_EVENT, read);
      window.removeEventListener(HYDRATED_EVENT, read);
    };
  }, [frontier, bookId]);

  // Abo-Vorschau (lva#334): `null`, solange das Gesprächsmodell kein Abo-Modell ist.
  const [subscription, setSubscription] = useState<
    "on" | "off" | "reconfirm" | "not_released" | null
  >(null);
  const [subscriptionReason, setSubscriptionReason] = useState<string | null>(null);
  const [riskOpen, setRiskOpen] = useState(false);
  const [riskSaving, setRiskSaving] = useState(false);
  const [riskError, setRiskError] = useState<string | null>(null);
  const refreshSubscription = useCallback(async () => {
    try {
      const status = await commands.bookTalkPreviewStatus();
      if (status.status !== "ok" || !status.data.applies) return setSubscription(null);
      const { enabled, state, blocked } = status.data;
      setSubscriptionReason(blocked ?? null);
      setSubscription(
        !enabled
          ? "off"
          : state === "reconfirm" || state === "not_released"
            ? state
            : "on",
      );
    } catch {
      setSubscription(null);
    }
  }, []);
  useEffect(() => {
    void refreshSubscription();
  }, [refreshSubscription, settings?.llm_active_model_id, settings?.function_models]);
  const toggleSubscription = async () => {
    if (subscription === "on" || subscription === "not_released") {
      await commands.changeBookTalkSubscriptionPreviewSetting(false, false);
      return refreshSubscription();
    }
    setRiskError(null);
    setRiskOpen(true);
  };
  const confirmSubscription = async () => {
    setRiskSaving(true);
    try {
      const res = await commands.changeBookTalkSubscriptionPreviewSetting(true, true);
      if (res.status === "ok") setRiskOpen(false);
      else setRiskError(res.error);
      await refreshSubscription();
    } finally {
      setRiskSaving(false);
    }
  };

  const talk = useBookTalk({ port, bookId, frontier });
  const voiceRef = useRef<ReturnType<typeof useBookVoice> | null>(null);
  const enabled = settings?.book_talk_enabled ?? false;
  const found = bookModelOf(settings);

  // Cloud-Modell ohne Freigabe: erst fragen, dann senden (Plan §3.8, §11.4).
  // Nur mit eingeschaltetem Schalter; ohne ihn geht nichts an ein Modell.
  // Eben erteilte Freigaben gelten sofort, auch ehe die Einstellungen neu gerendert sind.
  const granted = useRef(new Set<string>());
  const guarded = useCallback(
    (resume: () => void): boolean => {
      const needs = pendingConsent(settings);
      if (!enabled || !needs || granted.current.has(consentKeyOf(needs))) return true;
      setConsent({ resume });
      return false;
    },
    [enabled, settings],
  );
  const ask = (question: string, input: "text" | "voice") => {
    if (guarded(() => void talk.ask(question, input))) void talk.ask(question, input);
  };
  const voice = useBookVoice({
    port,
    bookId,
    talk,
    frontier,
    host,
    keyboard,
    canStart: () => guarded(() => voiceRef.current?.toggle()),
  });
  voiceRef.current = voice;

  const jump = (c: BookTalkCitation) => {
    host.jumpTo(c);
    onNavigate?.();
  };
  const readFrom = (c: BookTalkCitation) => {
    // Die Wiedergabe beginnt am Beleg; ein laufendes Gespräch endet dafür.
    if (voice.phase !== "off" || voice.readingPaused) voice.end(false);
    host.readFrom(c.start);
    onNavigate?.();
  };
  const pausedAt = voice.readingPaused ? host.position() : null;

  return (
    <>
      <BookTalkColumn
        talk={talk}
        voice={voice}
        limit={scopeLimit(text, boundary, chapterStarts)}
        pausedPage={pausedAt !== null ? pageAt(text, pausedAt) : null}
        enabled={enabled}
        onEnabled={(on) => void updateSetting("book_talk_enabled", on)}
        cloud={
          found && (settings?.book_talk_cloud_consent ?? []).includes(consentKeyOf(found))
            ? found.model.label || found.model.remote_id
            : null
        }
        subscription={subscription}
        subscriptionReason={subscriptionReason}
        onSubscription={() => void toggleSubscription()}
        onRevoke={() => {
          if (!found) return;
          granted.current.delete(consentKeyOf(found));
          void updateSetting(
            "book_talk_cloud_consent",
            withoutConsent(settings?.book_talk_cloud_consent, consentKeyOf(found)),
          );
        }}
        onAsk={ask}
        onJump={jump}
        onReadFrom={readFrom}
        onConsent={(turn) => setConsent({ resume: () => talk.askAgain(turn) })}
      />
      <SubscriptionPreviewDialog
        open={riskOpen}
        reconfirm={subscription === "reconfirm"}
        saving={riskSaving}
        error={riskError}
        onConfirm={() => void confirmSubscription()}
        onClose={() => setRiskOpen(false)}
      />
      <ConsentDialog
        open={consent !== null}
        onClose={() => setConsent(null)}
        onGranted={() => {
          const wait = consent;
          setConsent(null);
          if (found) granted.current.add(consentKeyOf(found));
          wait?.resume();
        }}
      />
    </>
  );
}
