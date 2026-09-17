import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
/**
 * Transport glyphs, straight from the design system
 * (references/components.md → Media-Controls). Filled shapes are the stated
 * exception to the line-icon set, and Unicode play/pause characters are ruled
 * out because Windows renders them as emoji, uncolourable via `color`.
 */
const GLYPHS = {
  play: <path d="M8.5 5.2 19 12 8.5 18.8Z" />,
  pause: (
    <>
      <rect x="7.4" y="5.2" width="3.6" height="13.6" rx="1.1" />
      <rect x="13" y="5.2" width="3.6" height="13.6" rx="1.1" />
    </>
  ),
  stop: <rect x="6.2" y="6.2" width="11.6" height="11.6" rx="1.6" />,
  prev: (
    <>
      <rect x="5.4" y="5.4" width="2.6" height="13.2" rx="1.1" />
      <path d="M19 5.4 19 18.6 9.2 12Z" />
    </>
  ),
  next: (
    <>
      <path d="M5 5.4 5 18.6 14.8 12Z" />
      <rect x="16" y="5.4" width="2.6" height="13.2" rx="1.1" />
    </>
  ),
  /* Ring stays hollow so the number inside it keeps its contrast. */
  back: (
    <>
      <path
        d="M8 6.1A8 8 0 1 0 16 6.1"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
      <path d="M8.6 2.4 8.6 9.8 3.9 6.1Z" />
    </>
  ),
  volume: (
    <>
      <path d="M4.5 9.3h3.2L12 5.6v12.8L7.7 14.7H4.5Z" />
      <path
        d="M15.2 9.1a4 4 0 0 1 0 5.8M17.6 6.9a7.2 7.2 0 0 1 0 10.2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </>
  ),
  mute: (
    <>
      <path d="M4.5 9.3h3.2L12 5.6v12.8L7.7 14.7H4.5Z" />
      <path
        d="M15.4 9.8 20 14.4M20 9.8 15.4 14.4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
      />
    </>
  ),
} as const;

export const Glyph: React.FC<{
  name: keyof typeof GLYPHS;
  mirrored?: boolean;
}> = ({ name, mirrored = false }) => (
  <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
    {mirrored ? (
      <g transform="translate(24,0) scale(-1,1)">{GLYPHS[name]}</g>
    ) : (
      GLYPHS[name]
    )}
  </svg>
);

/** How far the skip buttons jump. */
const SKIP_SECONDS = 15;

/** Selectable playback speeds; 1 is the default. */
const PLAYBACK_RATES = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 2] as const;

/** Last speed the user picked, shared across players within a session. */
let lastPlaybackRate = 1;
/** Same for volume — set it once, every player in the session follows. */
let lastVolume = 1;

const formatRate = (rate: number): string => `${rate}×`;

/** Steuerung von aussen — z. B. ein Zeitstempel im Transkript, der an die
 * Stelle springen und dort abspielen soll. Als Prop statt forwardRef, damit
 * die Komponente nicht umgehuellt (und komplett neu eingerueckt) wird. */
export interface AudioPlayerHandle {
  /** Springt zu `seconds` und spielt ab; laedt die Quelle vorher, falls noetig. */
  playAt: (seconds: number) => void;
}

interface AudioPlayerProps {
  /** Audio source URL. If not provided, onLoadRequest must be provided. */
  src?: string;
  /** Griff fuer Sprungmarken (siehe `AudioPlayerHandle`). */
  controlRef?: React.Ref<AudioPlayerHandle>;
  /** Called when play is clicked and no src is loaded yet. Should return the audio URL. */
  onLoadRequest?: () => Promise<string | null>;
  className?: string;
  autoPlay?: boolean;
}

interface AudioPlayerGroupContextValue {
  requestPlayback: (audio: HTMLAudioElement) => void;
  releasePlayback: (audio: HTMLAudioElement) => void;
}

const AudioPlayerGroupContext =
  createContext<AudioPlayerGroupContextValue | null>(null);

export const AudioPlayerGroup: React.FC<React.PropsWithChildren> = ({
  children,
}) => {
  const activeAudioRef = useRef<HTMLAudioElement | null>(null);
  const value = useMemo<AudioPlayerGroupContextValue>(
    () => ({
      requestPlayback: (audio) => {
        if (activeAudioRef.current !== audio) activeAudioRef.current?.pause();
        activeAudioRef.current = audio;
      },
      releasePlayback: (audio) => {
        if (activeAudioRef.current === audio) activeAudioRef.current = null;
      },
    }),
    [],
  );

  return (
    <AudioPlayerGroupContext.Provider value={value}>
      {children}
    </AudioPlayerGroupContext.Provider>
  );
};

export const AudioPlayer: React.FC<AudioPlayerProps> = ({
  src: initialSrc,
  controlRef,
  onLoadRequest,
  className = "",
  autoPlay = false,
}) => {
  const group = useContext(AudioPlayerGroupContext);
  const [isPlaying, setIsPlaying] = useState(false);
  const [duration, setDuration] = useState(0);
  const [currentTime, setCurrentTime] = useState(0);
  const [isDragging, setIsDragging] = useState(false);
  const [loadedSrc, setLoadedSrc] = useState<string | null>(initialSrc ?? null);
  const [isLoading, setIsLoading] = useState(false);
  const [playbackRate, setPlaybackRate] = useState(lastPlaybackRate);
  const [isRateMenuOpen, setIsRateMenuOpen] = useState(false);
  const rateMenuRef = useRef<HTMLDivElement>(null);
  const [volume, setVolume] = useState(lastVolume);
  const [isVolumeOpen, setIsVolumeOpen] = useState(false);
  const volumeRef = useRef<HTMLDivElement>(null);

  const audioRef = useRef<HTMLAudioElement>(null);
  const src = loadedSrc;
  const animationRef = useRef<number>();
  const dragTimeRef = useRef<number>(0);

  // Use refs to avoid stale closures in animation loop
  const isPlayingRef = useRef(false);
  const isDraggingRef = useRef(false);

  // Keep refs in sync with state
  useEffect(() => {
    isPlayingRef.current = isPlaying;
  }, [isPlaying]);

  useEffect(() => {
    isDraggingRef.current = isDragging;
  }, [isDragging]);

  // Stable animation loop with no dependencies
  const tick = useCallback(() => {
    if (audioRef.current && !isDraggingRef.current) {
      const time = audioRef.current.currentTime;
      setCurrentTime(time);
    }

    if (isPlayingRef.current) {
      animationRef.current = requestAnimationFrame(tick);
    }
  }, []); // Empty dependency array is key!

  // Manage animation loop lifecycle
  useEffect(() => {
    if (isPlaying && !isDragging) {
      // Only start if not already running
      if (!animationRef.current) {
        animationRef.current = requestAnimationFrame(tick);
      }
    } else {
      // Stop animation loop
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    }

    return () => {
      if (animationRef.current) {
        cancelAnimationFrame(animationRef.current);
        animationRef.current = undefined;
      }
    };
  }, [isPlaying, isDragging, tick]);

  // Keep the audio element in sync with the selected speed and volume
  useEffect(() => {
    if (audioRef.current) audioRef.current.playbackRate = playbackRate;
  }, [playbackRate, loadedSrc]);

  useEffect(() => {
    if (audioRef.current) audioRef.current.volume = volume;
  }, [volume, loadedSrc]);

  // Close the speed menu when clicking outside of it
  useEffect(() => {
    if (!isRateMenuOpen) return;

    const handleClickOutside = (event: MouseEvent) => {
      if (
        rateMenuRef.current &&
        !rateMenuRef.current.contains(event.target as Node)
      ) {
        setIsRateMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isRateMenuOpen]);

  useEffect(() => {
    if (!isVolumeOpen) return;
    const handleClickOutside = (event: MouseEvent) => {
      if (
        volumeRef.current &&
        !volumeRef.current.contains(event.target as Node)
      ) {
        setIsVolumeOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isVolumeOpen]);

  // Audio event handlers
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    const handleLoadedMetadata = () => {
      setDuration(audio.duration || 0);
      setCurrentTime(0);
    };

    const handleEnded = () => {
      group?.releasePlayback(audio);
      setIsPlaying(false);
      setCurrentTime(audio.duration || 0);
    };

    const handlePlay = () => {
      group?.requestPlayback(audio);
      setIsPlaying(true);
    };
    const handlePause = () => {
      group?.releasePlayback(audio);
      setIsPlaying(false);
    };

    audio.addEventListener("loadedmetadata", handleLoadedMetadata);
    audio.addEventListener("ended", handleEnded);
    audio.addEventListener("play", handlePlay);
    audio.addEventListener("pause", handlePause);

    return () => {
      group?.releasePlayback(audio);
      audio.removeEventListener("loadedmetadata", handleLoadedMetadata);
      audio.removeEventListener("ended", handleEnded);
      audio.removeEventListener("play", handlePlay);
      audio.removeEventListener("pause", handlePause);
    };
  }, [group]);

  // Auto-play when src becomes available (via onLoadRequest or autoPlay prop)
  const prevLoadedSrc = useRef<string | null>(null);
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;

    // Play when loadedSrc changes from null to a value (lazy load case)
    if (loadedSrc && !prevLoadedSrc.current && onLoadRequest) {
      audio.play().catch((error) => {
        console.error("Auto-play failed:", error);
      });
    }
    // Or when autoPlay is set with initial src
    else if (autoPlay && initialSrc && !prevLoadedSrc.current) {
      audio.play().catch((error) => {
        console.error("Auto-play failed:", error);
      });
    }

    prevLoadedSrc.current = loadedSrc;
  }, [loadedSrc, autoPlay, initialSrc, onLoadRequest]);

  // Global drag handlers
  const handleMouseUp = useCallback(() => {
    if (isDragging) {
      setIsDragging(false);
      if (audioRef.current) {
        audioRef.current.currentTime = dragTimeRef.current;
        setCurrentTime(dragTimeRef.current);
      }
    }
  }, [isDragging]);

  useEffect(() => {
    if (isDragging) {
      document.addEventListener("mouseup", handleMouseUp);
      document.addEventListener("touchend", handleMouseUp);

      return () => {
        document.removeEventListener("mouseup", handleMouseUp);
        document.removeEventListener("touchend", handleMouseUp);
      };
    }
  }, [isDragging, handleMouseUp]);

  // Cleanup blob URLs on unmount
  useEffect(() => {
    return () => {
      if (loadedSrc?.startsWith("blob:")) {
        URL.revokeObjectURL(loadedSrc);
      }
    };
  }, [loadedSrc]);

  useImperativeHandle(
    controlRef,
    () => ({
      playAt: async (seconds) => {
        const audio = audioRef.current;
        if (!audio) return;
        try {
          if (!src && onLoadRequest) {
            setIsLoading(true);
            const newSrc = await onLoadRequest();
            setIsLoading(false);
            if (!newSrc) return;
            setLoadedSrc(newSrc);
            // Erst nach dem Laden der Metadaten laesst sich sicher springen.
            audio.addEventListener(
              "loadedmetadata",
              () => {
                audio.currentTime = Math.max(0, seconds);
                void audio.play();
              },
              { once: true },
            );
            return;
          }
          audio.currentTime = Math.max(0, seconds);
          setCurrentTime(audio.currentTime);
          await audio.play();
        } catch (error) {
          console.error("Playback failed:", error);
        }
      },
    }),
    [src, onLoadRequest],
  );

  const togglePlay = async () => {
    const audio = audioRef.current;
    if (!audio) return;
    if (isLoading) return;

    try {
      if (isPlaying) {
        audio.pause();
      } else {
        // If no src loaded yet, request it
        if (!src && onLoadRequest) {
          setIsLoading(true);
          const newSrc = await onLoadRequest();
          setIsLoading(false);
          if (newSrc) {
            setLoadedSrc(newSrc);
            // Playback will be triggered by the useEffect watching loadedSrc
          }
        } else if (src) {
          await audio.play();
        }
      }
    } catch (error) {
      console.error("Playback failed:", error);
    }
  };

  const handleSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newTime = parseFloat(e.target.value);
    dragTimeRef.current = newTime;
    setCurrentTime(newTime);

    if (!isDragging && audioRef.current) {
      audioRef.current.currentTime = newTime;
    }
  };

  const handleSliderMouseDown = () => {
    setIsDragging(true);
  };

  const handleSliderTouchStart = () => {
    setIsDragging(true);
  };

  const skip = (seconds: number) => {
    const audio = audioRef.current;
    if (!audio || !isFinite(audio.duration)) return;
    const target = Math.min(
      Math.max(0, audio.currentTime + seconds),
      audio.duration,
    );
    audio.currentTime = target;
    setCurrentTime(target);
  };

  /** Stop is "halt and rewind", which is what makes it distinct from pause. */
  const stop = () => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.pause();
    audio.currentTime = 0;
    setCurrentTime(0);
  };

  const handleSelectVolume = (value: number) => {
    lastVolume = value;
    setVolume(value);
  };

  const handleSelectRate = (rate: number) => {
    lastPlaybackRate = rate;
    setPlaybackRate(rate);
    setIsRateMenuOpen(false);
  };

  const formatTime = (time: number): string => {
    if (!isFinite(time)) return "0:00";

    const minutes = Math.floor(time / 60);
    const seconds = Math.floor(time % 60);
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  };

  // Fix playhead positioning with better edge case handling
  const getProgressPercent = (): number => {
    if (duration <= 0) return 0;

    // Handle the end case - if we're within 0.1 seconds of the end, show 100%
    if (duration - currentTime < 0.1) return 100;

    const percent = (currentTime / duration) * 100;
    return Math.min(100, Math.max(0, percent));
  };

  const progressPercent = getProgressPercent();

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      <audio ref={audioRef} src={src ?? undefined} preload="metadata" />

      {/* Transport per design system: round glyph buttons, exactly one
          primary (play/pause) per player, never text inside the button. */}
      <div className="mediabar mediabar--start">
        <button
          type="button"
          className="mbtn mbtn--sm"
          onClick={() => skip(-SKIP_SECONDS)}
          aria-label={`${SKIP_SECONDS} Sekunden zurück`}
        >
          <Glyph name="back" />
        </button>
        <button
          type="button"
          className="mbtn mbtn--primary"
          onClick={togglePlay}
          disabled={isLoading}
          aria-label={isPlaying ? "Pause" : "Wiedergabe"}
        >
          <Glyph name={isPlaying ? "pause" : "play"} />
        </button>
        <button
          type="button"
          className="mbtn mbtn--sm"
          onClick={() => skip(SKIP_SECONDS)}
          aria-label={`${SKIP_SECONDS} Sekunden vor`}
        >
          <Glyph name="back" mirrored />
        </button>
        <button
          type="button"
          className="mbtn mbtn--sm"
          onClick={stop}
          aria-label="Stopp"
        >
          <Glyph name="stop" />
        </button>
      </div>

      <div className="flex-1 flex items-center gap-2">
        <span className="text-xs text-text/60 min-w-[30px] tabular-nums">
          {formatTime(currentTime)}
        </span>

        <input
          type="range"
          min="0"
          max={duration || 0}
          step="0.01"
          value={currentTime}
          onChange={handleSeek}
          onMouseDown={handleSliderMouseDown}
          onTouchStart={handleSliderTouchStart}
          className={`flex-1 h-1 rounded-lg appearance-none cursor-pointer focus:outline-none focus:ring-1 focus:ring-logo-primary ${progressPercent >= 99.5 ? "[&::-webkit-slider-thumb]:translate-x-0.5 [&::-moz-range-thumb]:translate-x-0.5" : ""}`}
          style={{
            // Signalgelb aus dem Token-Satz statt Handys Rosa, das hier aus
            // dem Fork uebriggeblieben war.
            background: `linear-gradient(to right, var(--color-logo-primary) 0%, var(--color-logo-primary) ${progressPercent}%, rgba(128, 128, 128, 0.2) ${progressPercent}%, rgba(128, 128, 128, 0.2) 100%)`,
          }}
        />

        <span className="text-xs text-text/60 min-w-[30px] tabular-nums">
          {formatTime(duration)}
        </span>
      </div>

      <div className="relative" ref={volumeRef}>
        <button
          type="button"
          className="mbtn mbtn--sm"
          onClick={() => setIsVolumeOpen((open) => !open)}
          aria-label="Lautstärke"
          aria-expanded={isVolumeOpen}
        >
          <Glyph name={volume === 0 ? "mute" : "volume"} />
        </button>
        {isVolumeOpen && (
          <div className="absolute end-0 bottom-full z-50 mb-1 w-40 rounded-md border border-mid-gray/80 bg-background shadow-lg px-3 py-2">
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              value={volume}
              onChange={(e) => handleSelectVolume(parseFloat(e.target.value))}
              aria-label="Lautstärke"
              className="w-full h-1 rounded-lg appearance-none cursor-pointer focus:outline-none focus:ring-1 focus:ring-logo-primary"
              style={{
                background: `linear-gradient(to right, var(--color-logo-primary) 0%, var(--color-logo-primary) ${volume * 100}%, rgba(128, 128, 128, 0.2) ${volume * 100}%, rgba(128, 128, 128, 0.2) 100%)`,
              }}
            />
            <p className="mt-1 text-xs text-text/60 tabular-nums text-center">
              {Math.round(volume * 100)}%
            </p>
          </div>
        )}
      </div>

      <div className="relative" ref={rateMenuRef}>
        <button
          type="button"
          onClick={() => setIsRateMenuOpen((open) => !open)}
          className={`px-1.5 py-0.5 text-xs font-semibold tabular-nums rounded-md border transition-colors cursor-pointer ${
            playbackRate === 1
              ? "border-mid-gray/80 bg-mid-gray/10 text-text/70 hover:border-logo-primary hover:bg-logo-primary/10"
              : "border-logo-primary bg-logo-primary/10 text-text"
          }`}
          aria-label="Playback speed"
          aria-haspopup="listbox"
          aria-expanded={isRateMenuOpen}
        >
          {formatRate(playbackRate)}
        </button>

        {isRateMenuOpen && (
          <div
            role="listbox"
            className="absolute right-0 bottom-full z-50 mb-1 rounded-md border border-mid-gray/80 bg-background shadow-lg py-1"
          >
            {PLAYBACK_RATES.map((rate) => (
              <button
                key={rate}
                type="button"
                role="option"
                aria-selected={rate === playbackRate}
                onClick={() => handleSelectRate(rate)}
                className={`block w-full px-3 py-1 text-xs text-start tabular-nums whitespace-nowrap transition-colors cursor-pointer hover:bg-logo-primary/10 ${
                  rate === playbackRate
                    ? "text-logo-primary font-semibold"
                    : "text-text"
                }`}
              >
                {formatRate(rate)}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
