import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, ExternalLink, Play } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { YoutubeSource } from "@/bindings";
import { usePersistentState } from "@/hooks/usePersistentState";
import { Button } from "../../../ui/Button";
import {
  embedUrl,
  loadYoutubeApi,
  watchUrl,
  type YtPlayer,
} from "./youtubeApi";

/** Hoehe des Videobereichs in Pixeln: Grenzen, Vorgabe (16:9 bei ~480 px Breite), Tastenschritt. */
export const STAGE_MIN = 120;
export const STAGE_MAX = 560;
export const STAGE_DEFAULT = 270;
const KEY_STEP = 16;

const clampHeight = (value: number) =>
  Math.min(Math.max(Math.round(value), STAGE_MIN), STAGE_MAX);

const isOpenState = (value: string) => value === "open" || value === "closed";
const isHeightText = (value: string) => /^\d{2,4}$/.test(value);

/** Was die Besprechung vom Player steuern darf (Zeitsprung aus dem Transkript). */
export interface YoutubePanelHandle {
  /** Ab `ms` abspielen; laedt den Player zuerst, falls er noch nicht laeuft. */
  seek(ms: number): void;
  /** Haelt das Video an (nur, wenn der Player laeuft). */
  pause(): void;
}

type Phase = "idle" | "loading" | "ready" | "error";

/** Fehlercodes des Players (onError) mit eigener Meldung; alles andere: allgemein. */
const PLAYER_ERROR_CODES = new Set([2, 5, 100, 101, 150, 153]);

interface Props {
  source: YoutubeSource;
  /** Die Videodauer (Sekunden), sobald der Player sie kennt; hoechstens einmal je Video. */
  onDuration?: (seconds: number) => void;
}

/**
 * Der YouTube-Player in der Inhaltsspalte (#66 AK3): oben in der Arbeitsflaeche,
 * einklappbar, mit einem Griff fuer die Hoehe; beides bleibt ueber Neuladen und
 * Neustart erhalten. Offizieller Einbett-Player, ohne Eingriff in die Werbung.
 *
 * Laedt erst, wenn der Nutzer es will ("Video abspielen" oder ein Klick auf eine
 * Zeitmarke): vorher verbindet sich nichts mit YouTube (Offline-Pfad, QG5).
 */
export const YoutubePlayerPanel = forwardRef<YoutubePanelHandle, Props>(
  ({ source, onDuration }, ref) => {
    const { t } = useTranslation();
    const onDurationRef = useRef(onDuration);
    onDurationRef.current = onDuration;
    const durationSent = useRef<string | null>(null);
    // Die Sprache darf den laufenden Player nicht neu aufbauen: der Effekt liest `t` ueber eine Ref.
    const tRef = useRef(t);
    tRef.current = t;
    const [openState, setOpenState] = usePersistentState<"open" | "closed">(
      "youtube.playerOpen",
      "open",
      isOpenState,
    );
    const [heightText, setHeightText] = usePersistentState<string>(
      "youtube.playerHeight",
      String(STAGE_DEFAULT),
      isHeightText,
    );
    const open = openState === "open";
    const height = clampHeight(Number(heightText));
    const setHeight = useCallback(
      (value: number) => setHeightText(String(clampHeight(value))),
      [setHeightText],
    );

    const [phase, setPhase] = useState<Phase>("idle");
    const [errorText, setErrorText] = useState<string | null>(null);
    const [wanted, setWanted] = useState(false);
    // Zaehlt die Versuche: ein neuer Versuch baut Rahmen und Player neu auf.
    const [attempt, setAttempt] = useState(0);
    const iframeRef = useRef<HTMLIFrameElement | null>(null);
    const playerRef = useRef<YtPlayer | null>(null);
    const pendingMs = useRef<number | null>(null);
    const mounted = useRef(true);
    const phaseRef = useRef<Phase>("idle");
    phaseRef.current = phase;
    const openRef = useRef(open);
    openRef.current = open;

    useEffect(() => {
      mounted.current = true;
      return () => {
        mounted.current = false;
      };
    }, []);

    const src = useMemo(
      () => embedUrl(source.video_id, source.start_s, window.location.origin),
      [source.video_id, source.start_s],
    );

    const reportDuration = () => {
      const seconds = playerRef.current?.getDuration?.() ?? 0;
      if (seconds > 0 && durationSent.current !== source.video_id) {
        durationSent.current = source.video_id;
        onDurationRef.current?.(seconds);
      }
    };

    // Den Player an den Rahmen haengen, sobald der Nutzer ihn will. Kein
    // `destroy()`: es entfernt den Rahmen selbst, und React tut es beim Abbau
    // noch einmal.
    useEffect(() => {
      if (!wanted) return;
      let cancelled = false;
      setPhase("loading");
      setErrorText(null);
      loadYoutubeApi()
        .then((YT) => {
          const frame = iframeRef.current;
          if (cancelled || !mounted.current || !frame) return;
          playerRef.current = new YT.Player(frame, {
            events: {
              onStateChange: (event) => {
                // Nach dem Start kennt der Player die Dauer (vorher meldet er 0).
                if (cancelled || !mounted.current || event.data !== 1) return;
                reportDuration();
              },
              onReady: () => {
                if (cancelled || !mounted.current) return;
                setPhase("ready");
                const at = pendingMs.current;
                pendingMs.current = null;
                const player = playerRef.current;
                if (player) {
                  if (at !== null) player.seekTo(at / 1000, true);
                  // Wer "abspielen" oder eine Zeitmarke gedrueckt hat, will es sehen.
                  player.playVideo();
                }
              },
              onError: (event) => {
                if (cancelled || !mounted.current) return;
                const code = PLAYER_ERROR_CODES.has(event.data)
                  ? String(event.data)
                  : "generic";
                setErrorText(
                  tRef.current(`meetings.youtube.player.errors.${code}`),
                );
                setPhase("error");
              },
            },
          });
        })
        .catch(() => {
          if (cancelled || !mounted.current) return;
          setErrorText(tRef.current("meetings.youtube.player.errors.api"));
          setPhase("error");
        });
      return () => {
        cancelled = true;
      };
    }, [wanted, attempt]);

    useImperativeHandle(
      ref,
      () => ({
        seek(ms: number) {
          // Ein Zeitsprung ist ein bewusster Klick: Bereich aufklappen, Player laden.
          if (!openRef.current) setOpenState("open");
          const player = playerRef.current;
          if (phaseRef.current === "ready" && player) {
            player.seekTo(ms / 1000, true);
            player.playVideo();
            return;
          }
          pendingMs.current = ms;
          if (phaseRef.current === "error") {
            playerRef.current = null;
            setAttempt((a) => a + 1);
          }
          setWanted(true);
        },
        pause() {
          if (phaseRef.current === "ready") playerRef.current?.pauseVideo();
        },
      }),
      [setOpenState],
    );

    const toggle = () => {
      if (open && phase === "ready") playerRef.current?.pauseVideo();
      setOpenState(open ? "closed" : "open");
    };

    const retry = () => {
      playerRef.current = null;
      setAttempt((a) => a + 1);
    };

    // Hoehe: ziehen, Pfeil hoch/runter, Pos1/Ende, Doppelklick = Vorgabe.
    const dragStart = useRef<{ y: number; h: number } | null>(null);
    const onHandleKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
      let next: number | null = null;
      if (event.key === "ArrowUp") next = height - KEY_STEP;
      else if (event.key === "ArrowDown") next = height + KEY_STEP;
      else if (event.key === "Home") next = STAGE_MIN;
      else if (event.key === "End") next = STAGE_MAX;
      if (next === null) return;
      event.preventDefault();
      setHeight(next);
    };

    const showFrame = wanted;
    const overlayVisible = phase !== "ready";

    return (
      <section
        data-testid="yt-panel"
        data-phase={phase}
        data-open={open ? "true" : "false"}
        aria-label={t("meetings.youtube.player.region")}
        className="rounded-lg border border-mid-gray/20 bg-background"
      >
        <div className="flex h-9 items-center gap-2 px-2">
          <button
            type="button"
            data-testid="yt-toggle"
            aria-expanded={open}
            aria-label={
              open
                ? t("meetings.youtube.player.collapse")
                : t("meetings.youtube.player.expand")
            }
            title={
              open
                ? t("meetings.youtube.player.collapse")
                : t("meetings.youtube.player.expand")
            }
            onClick={toggle}
            className="flex min-w-0 cursor-pointer items-center gap-1 rounded-md px-1 text-sm font-medium text-text/80 hover:bg-mid-gray/15 hover:text-text"
          >
            {open ? (
              <ChevronDown width={16} height={16} aria-hidden="true" />
            ) : (
              <ChevronRight width={16} height={16} aria-hidden="true" />
            )}
            {t("meetings.youtube.player.title")}
          </button>
          <span
            className="min-w-0 flex-1 truncate text-xs text-text/60"
            data-testid="yt-channel"
            title={source.title}
          >
            {source.channel || source.title}
          </span>
          <button
            type="button"
            data-testid="yt-open-external"
            aria-label={t("meetings.youtube.player.openExternal")}
            title={t("meetings.youtube.player.openExternal")}
            onClick={() =>
              void openUrl(watchUrl(source.video_id, source.start_s))
            }
            className="cursor-pointer rounded-md p-1 text-text/60 hover:bg-mid-gray/20 hover:text-text"
          >
            <ExternalLink width={16} height={16} aria-hidden="true" />
          </button>
        </div>

        {/* Der Bereich bleibt beim Einklappen im Baum (verborgen): so geht die
            Position nicht verloren. Eingeklappt wird pausiert. */}
        <div hidden={!open}>
          <div
            data-testid="yt-stage"
            style={{ height: `${height}px` }}
            className="relative flex justify-center bg-black"
          >
            <div
              className="relative h-full max-w-full"
              style={{ aspectRatio: "16 / 9" }}
            >
              {showFrame && (
                <iframe
                  key={attempt}
                  ref={iframeRef}
                  data-testid="yt-iframe"
                  src={src}
                  title={source.title}
                  className="absolute inset-0 h-full w-full border-0"
                  allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
                  allowFullScreen
                  referrerPolicy="strict-origin-when-cross-origin"
                />
              )}
              {overlayVisible && (
                <div
                  data-testid="yt-overlay"
                  className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/90 p-3 text-center text-white"
                >
                  {phase === "idle" && (
                    <>
                      <p className="line-clamp-2 text-sm font-medium">
                        {source.title}
                      </p>
                      <Button
                        size="sm"
                        data-testid="yt-play"
                        onClick={() => setWanted(true)}
                      >
                        <Play width={14} height={14} aria-hidden="true" />
                        {t("meetings.youtube.player.play")}
                      </Button>
                      <p className="max-w-xs text-xs text-white/70">
                        {t("meetings.youtube.player.privacy")}
                      </p>
                    </>
                  )}
                  {phase === "loading" && (
                    <p
                      className="text-sm text-white/80"
                      data-testid="yt-loading"
                      role="status"
                    >
                      {t("meetings.youtube.player.loading")}
                    </p>
                  )}
                  {phase === "error" && (
                    <>
                      <p
                        className="max-w-sm text-sm"
                        data-testid="yt-error"
                        role="alert"
                      >
                        {errorText}
                      </p>
                      <div className="flex gap-2">
                        <Button
                          size="sm"
                          variant="secondary"
                          data-testid="yt-retry"
                          onClick={retry}
                        >
                          {t("meetings.youtube.player.retry")}
                        </Button>
                      </div>
                    </>
                  )}
                </div>
              )}
            </div>
          </div>
          <div
            role="separator"
            aria-orientation="horizontal"
            aria-valuenow={height}
            aria-valuemin={STAGE_MIN}
            aria-valuemax={STAGE_MAX}
            aria-label={t("meetings.youtube.player.resize")}
            title={t("meetings.youtube.player.resize")}
            tabIndex={0}
            data-testid="yt-resize"
            className="rec-hsplit"
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.currentTarget.setPointerCapture(event.pointerId);
              dragStart.current = { y: event.clientY, h: height };
              event.preventDefault();
            }}
            onPointerMove={(event) => {
              const from = dragStart.current;
              if (from) setHeight(from.h + (event.clientY - from.y));
            }}
            onPointerUp={() => {
              dragStart.current = null;
            }}
            onPointerCancel={() => {
              dragStart.current = null;
            }}
            onKeyDown={onHandleKey}
            onDoubleClick={() => setHeight(STAGE_DEFAULT)}
          />
        </div>
      </section>
    );
  },
);

YoutubePlayerPanel.displayName = "YoutubePlayerPanel";
