import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Mic, Pause, Send, Square } from "lucide-react";
import type { BookTalkCitation } from "@/bindings";
import { CitationCard } from "./CitationCard";
import { BOOKTALK_ROOT_ATTR } from "./keys";
import { ScopeBar } from "./ScopeBar";
import { citationsOf, type Turn } from "./turns";
import type { ScopeLimit } from "./pages";
import type { BookTalkState } from "./useBookTalk";
import type { BookVoice } from "./useBookVoice";
import "./booktalk.css";

const SUGGESTIONS = ["recap", "who", "last"] as const;

/** Gründe, bei denen die Antwort nur Fundstellen sind und ein Hinweis erklärt, warum. */
const KNOWN_REASONS = new Set([
  "no_model",
  "no_hits",
  "no_frontier",
  "budget",
  "offline",
  "timeout",
  "llm_failed",
  "consent_required",
]);

export interface BookTalkColumnProps {
  talk: BookTalkState;
  /** `null`, wenn kein Mikrofon-Zugang besteht (nur Text). */
  voice: BookVoice | null;
  /** Konkrete Lesegrenze (Seite, Kapitel oder Anfang). */
  limit: ScopeLimit;
  /** Seite, an der das Vorlesen für das Gespräch pausiert wurde. */
  pausedPage: number | null;
  /** Schalter `book_talk_enabled`: aus = Fundstellen-Modus mit Hinweis. */
  enabled: boolean;
  /** Schalter „KI-Antworten (Vorschau)“ umlegen. */
  onEnabled(on: boolean): void;
  /** Cloud-Modell mit Freigabe (Name) für den Menüpunkt „Freigabe widerrufen“; sonst `null`. */
  cloud: string | null;
  onRevoke(): void;
  /** Abo-Vorschau (lva#334): Zustand des Schalters, `null` ohne Abo-Modell. */
  subscription?: "on" | "off" | "reconfirm" | "not_released" | null;
  /** Grund, wenn `subscription` „not_released“ ist (`measurement_missing` …). */
  subscriptionReason?: string | null;
  onSubscription?(): void;
  /** Frage stellen; die Hülle fragt vorher nach der Cloud-Freigabe. */
  onAsk(question: string, input: "text" | "voice"): void;
  onJump(c: BookTalkCitation): void;
  onReadFrom(c: BookTalkCitation): void;
  /** Freigabe für ein Cloud-Modell prüfen (Antwort `consent_required`). */
  onConsent(turn: Turn): void;
}

/**
 * Gesprächsspalte (lva#309, G2): Bereichsknopf, Verlauf mit Belegchips und
 * Belegkarte, Vorschläge, Eingabe mit Sprechen-Knopf, Textfeld und Senden.
 * Nur Darstellung; Zustand und Anfragen kommen aus `useBookTalk` und `useBookVoice`.
 */
export function BookTalkColumn({
  talk,
  voice,
  limit,
  pausedPage,
  enabled,
  onEnabled,
  cloud,
  onRevoke,
  subscription = null,
  subscriptionReason = null,
  onSubscription,
  onAsk,
  onJump,
  onReadFrom,
  onConsent,
}: BookTalkColumnProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState("");
  const [open, setOpen] = useState<{ turn: string; cite: BookTalkCitation } | null>(
    null,
  );
  const [announce, setAnnounce] = useState("");
  const log = useRef<HTMLDivElement>(null);
  const announced = useRef<string | null>(null);

  // Laufender Text bricht um und scrollt mit, nie abgeschnitten.
  const last = talk.turns[talk.turns.length - 1];
  const progress = `${talk.turns.length}:${last?.sentences.length ?? 0}:${last?.status ?? ""}`;
  useEffect(() => {
    const el = log.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [progress]);

  // Ansage „Antwort fertig, n Belege“ für Screenreader, einmal je Antwort.
  useEffect(() => {
    if (!last || last.status !== "done" || announced.current === last.id) return;
    announced.current = last.id;
    const count = citationsOf(last).length;
    setAnnounce(
      count > 0
        ? t("tts.bookTalk.voice.answerDone", { count })
        : t("tts.bookTalk.voice.answerDoneNone"),
    );
  }, [last, t]);

  const send = (text: string, input: "text" | "voice" = "text") => {
    const clean = text.trim();
    if (!clean || talk.busy || !talk.ready) return;
    setDraft("");
    onAsk(clean, input);
  };

  const phase = voice?.phase ?? "off";
  const live = phase !== "off";
  const stateText =
    phase === "starting"
      ? t("tts.bookTalk.voice.starting")
      : phase === "listening"
        ? t("tts.bookTalk.voice.listening")
        : phase === "hearing"
          ? t("tts.bookTalk.voice.hearing")
          : phase === "thinking"
            ? t("tts.bookTalk.voice.thinking")
            : phase === "speaking"
              ? t("tts.bookTalk.voice.speaking")
              : null;

  return (
    <div
      className="bt"
      data-testid="booktalk"
      role="region"
      aria-label={t("tts.bookTalk.region")}
      {...{ [BOOKTALK_ROOT_ATTR]: "" }}
    >
      <ScopeBar
        scope={talk.scope}
        limit={limit}
        onScope={(s) => (voice ? voice.setScope(s) : talk.setScope(s))}
        onClear={() => void talk.clear()}
        canClear={talk.turns.length > 0}
        preview={enabled}
        onPreview={onEnabled}
        cloud={cloud}
        onRevoke={onRevoke}
        subscription={subscription}
        subscriptionReason={subscriptionReason}
        onSubscription={onSubscription}
      />
      {!enabled && (
        <p className="bt__banner" role="note" data-testid="booktalk-disabled">
          {t("tts.bookTalk.disabled")}
        </p>
      )}
      <div className="bt__log" ref={log} data-testid="booktalk-log">
        {talk.turns.length === 0 && talk.ready && (
          <>
            <p className="bt__empty">{t("tts.bookTalk.empty")}</p>
            <div
              className="bt__suggest"
              role="group"
              aria-label={t("tts.bookTalk.suggest.label")}
              data-testid="booktalk-suggestions"
            >
              {SUGGESTIONS.map((key) => (
                <button
                  key={key}
                  type="button"
                  disabled={talk.busy || !talk.ready}
                  onClick={() => send(t(`tts.bookTalk.suggest.${key}`))}
                  data-testid={`booktalk-suggest-${key}`}
                >
                  {t(`tts.bookTalk.suggest.${key}`)}
                </button>
              ))}
            </div>
          </>
        )}
        {talk.turns.map((turn) => (
          <TurnView
            key={turn.id}
            turn={turn}
            open={open?.turn === turn.id ? open.cite : null}
            onOpen={(cite) =>
              setOpen(
                open?.turn === turn.id && open.cite.start === cite.start
                  ? null
                  : { turn: turn.id, cite },
              )
            }
            onJump={onJump}
            onReadFrom={onReadFrom}
            onAskAgain={() => talk.askAgain(turn)}
            onConsent={() => onConsent(turn)}
          />
        ))}
      </div>
      <span className="sr-only" role="status" aria-live="polite" data-testid="booktalk-announce">
        {announce}
      </span>

      {voice?.readingPaused && (
        <div className="bt__state bt__paused" data-testid="booktalk-paused">
          <Pause width={14} height={14} aria-hidden="true" />
          <span>
            {pausedPage !== null ? (
              <b>{t("tts.bookTalk.voice.pausedAt", { page: pausedPage })}</b>
            ) : (
              <b>{t("tts.bookTalk.voice.paused")}</b>
            )}
          </span>
          <button type="button" onClick={voice.resume} data-testid="booktalk-resume">
            {t("tts.bookTalk.voice.resume")}
          </button>
        </div>
      )}
      {stateText && (
        <div className="bt__state" data-testid="booktalk-state" aria-live="polite">
          {(phase === "listening" || phase === "hearing") && (
            <span className="bt__wave" aria-hidden="true">
              {[0.4, 0.8, 0.6, 1, 0.7].map((k, i) => (
                <i
                  key={i}
                  style={{
                    height: `${Math.max(4, Math.round(Math.min(1, (voice?.level ?? 0) * 8 * k) * 16))}px`,
                  }}
                />
              ))}
            </span>
          )}
          <span>{stateText}</span>
          {phase === "speaking" && voice && (
            <button type="button" onClick={voice.stopAnswer} data-testid="booktalk-stop-speaking">
              {t("tts.bookTalk.voice.stopSpeaking")}
            </button>
          )}
        </div>
      )}
      {voice?.error && (
        <p className="bt__banner" role="alert">
          {voice.error}
        </p>
      )}

      <form
        className="bt__composer"
        onSubmit={(e) => {
          e.preventDefault();
          send(draft);
        }}
      >
        {voice && (
          <button
            type="button"
            className={`bt__mic${live ? " bt__mic--live" : ""}`}
            aria-label={live ? t("tts.bookTalk.composer.micStop") : t("tts.bookTalk.composer.mic")}
            aria-pressed={live}
            aria-describedby="booktalk-keys"
            title={t("tts.bookTalk.composer.keys")}
            onClick={voice.toggle}
            data-testid="booktalk-mic"
          >
            {live ? (
              <Square width={20} height={20} aria-hidden="true" />
            ) : (
              <Mic width={24} height={24} aria-hidden="true" />
            )}
          </button>
        )}
        <input
          type="text"
          className="bt__field"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          aria-label={t("tts.bookTalk.composer.field")}
          placeholder={
            live
              ? t("tts.bookTalk.composer.placeholderVoice")
              : t("tts.bookTalk.composer.placeholder")
          }
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              send(draft);
            }
          }}
          data-testid="booktalk-input"
        />
        {talk.busy ? (
          <button
            type="button"
            className="bt__send"
            aria-label={t("tts.bookTalk.composer.stop")}
            title={t("tts.bookTalk.composer.stop")}
            onClick={() => (voice ? voice.stopAnswer() : talk.cancel())}
            data-testid="booktalk-cancel"
          >
            <Square width={16} height={16} aria-hidden="true" />
          </button>
        ) : (
          <button
            type="submit"
            className="bt__send"
            aria-label={t("tts.bookTalk.composer.send")}
            title={t("tts.bookTalk.composer.send")}
            disabled={!draft.trim() || !talk.ready}
            data-testid="booktalk-send"
          >
            <Send width={16} height={16} aria-hidden="true" />
          </button>
        )}
      </form>
      <span id="booktalk-keys" className="sr-only">
        {t("tts.bookTalk.composer.keys")}
      </span>
    </div>
  );
}

function TurnView({
  turn,
  open,
  onOpen,
  onJump,
  onReadFrom,
  onAskAgain,
  onConsent,
}: {
  turn: Turn;
  open: BookTalkCitation | null;
  onOpen(c: BookTalkCitation): void;
  onJump(c: BookTalkCitation): void;
  onReadFrom(c: BookTalkCitation): void;
  onAskAgain(): void;
  onConsent(): void;
}) {
  const { t } = useTranslation();
  const reason = turn.answer?.found_only ? (turn.answer.reason ?? "other") : null;
  const hintKey = reason && KNOWN_REASONS.has(reason) ? reason : "other";
  return (
    <div data-testid="booktalk-turn" data-status={turn.status}>
      <div className="bt__user" data-testid="booktalk-question">
        <div className="bt__meta">
          {turn.input === "voice" && <Mic width={11} height={11} aria-hidden="true" />}
          <span>{turn.input === "voice" ? t("tts.bookTalk.turn.spoken") : t("tts.bookTalk.turn.typed")}</span>
          {turn.scope === "book" && (
            <span className="bt__tag" data-testid="booktalk-turn-book">
              {t("tts.bookTalk.turn.wholeBook")}
            </span>
          )}
        </div>
        {turn.question}
      </div>
      <article
        className="bt__answer"
        aria-label={t("tts.bookTalk.turn.answer")}
        data-testid="booktalk-answer"
        data-found-only={turn.answer?.found_only ? "1" : undefined}
      >
        {reason && (
          <p className="bt__found" data-testid="booktalk-found">
            <b>{t("tts.bookTalk.found.title")}</b>
            {` · ${t(`tts.bookTalk.found.hint.${hintKey}`)}`}
            {reason === "consent_required" && (
              <button type="button" onClick={onConsent} data-testid="booktalk-consent-open">
                {t("tts.bookTalk.found.consentAction")}
              </button>
            )}
          </p>
        )}
        {(turn.answer ? !turn.answer.found_only : turn.sentences.length > 0) && (
          <p className="bt__preview" data-testid="booktalk-preview-tag">
            <span className="bt__tag">{t("tts.bookTalk.turn.preview")}</span>
            {` ${t("tts.bookTalk.turn.previewHint")}`}
          </p>
        )}
        {turn.answer?.preview && (
          <p className="bt__preview" data-testid="booktalk-subscription-tag">
            <span className="bt__tag">{t("tts.bookTalk.turn.preview")}</span>
            {` ${t("tts.bookTalk.turn.subscriptionHint")}`}
          </p>
        )}
        {turn.sentences.length === 0 && turn.status === "pending" && (
          <p className="bt__status">{t("tts.bookTalk.turn.thinking")}</p>
        )}
        {turn.sentences.map((s) => (
          <p
            key={s.index}
            className={s.general ? "bt__general" : undefined}
            title={s.general ? t("tts.bookTalk.turn.general") : undefined}
          >
            {s.text}
            {s.citations.map((c) => (
              <button
                key={`${c.start}:${c.end}`}
                type="button"
                className="bt__chip"
                aria-expanded={open?.start === c.start}
                aria-label={
                  c.page != null
                    ? t("tts.bookTalk.chip.aria", { page: c.page })
                    : t("tts.bookTalk.chip.ariaNoPage")
                }
                onClick={() => onOpen(c)}
                data-testid="booktalk-chip"
              >
                {c.page != null
                  ? t("tts.bookTalk.chip.label", { page: c.page })
                  : t("tts.bookTalk.chip.labelNoPage")}
              </button>
            ))}
          </p>
        ))}
        {turn.status === "cancelled" && (
          <p className="bt__status" data-testid="booktalk-cancelled">
            {turn.cancelReason === "scope"
              ? t("tts.bookTalk.turn.cancelledScope")
              : t("tts.bookTalk.turn.cancelled")}
            {turn.cancelReason === "scope" && (
              <button type="button" onClick={onAskAgain} data-testid="booktalk-ask-again">
                {t("tts.bookTalk.turn.askAgain")}
              </button>
            )}
          </p>
        )}
        {turn.status === "failed" && (
          <p className="bt__status" role="alert">
            {t("tts.bookTalk.turn.failed")}
          </p>
        )}
        {open && (
          <CitationCard
            citation={open}
            onJump={onJump}
            onReadFrom={onReadFrom}
            onClose={() => onOpen(open)}
          />
        )}
      </article>
    </div>
  );
}
