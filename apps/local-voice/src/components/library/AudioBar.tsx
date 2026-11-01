import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Pause, Play, RotateCcw, RotateCw, X } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import { IconButton } from "../ui/IconButton";
import { BookCover } from "./BookCover";
import { assetUrl } from "@/lib/library/books";
import {
  audioProgressKey,
  flushProgress,
  loadProgress,
  recordPosition,
} from "@/lib/library/progress";

/** Wie oft die Stelle beim Hören gesichert wird. */
const SAVE_EVERY_MS = 2000;

/**
 * Hörleiste für ein importiertes Hörbuch (lva#152). Sie setzt an der
 * gespeicherten Stelle an und sichert die Stelle beim Hören, Pausieren,
 * Springen und Schließen – gehörte Bereiche bleiben gehört.
 */
export const AudioBar: React.FC<{ book: LibraryBook; onClose(): void }> = ({
  book,
  onClose,
}) => {
  const { t } = useTranslation();
  const audio = useRef<HTMLAudioElement>(null);
  const lastSave = useRef(0);
  const key = audioProgressKey(book.id);
  const [playing, setPlaying] = useState(false);
  const skip = (seconds: number) => {
    const el = audio.current;
    if (el) el.currentTime = Math.max(0, el.currentTime + seconds);
  };
  const toggle = () => {
    const el = audio.current;
    if (!el) return;
    if (el.paused) void el.play().catch(() => {});
    else el.pause();
  };
  const src = assetUrl(book.audio_path);

  const persist = (force = false) => {
    const el = audio.current;
    if (!el || !Number.isFinite(el.duration)) return;
    const now = Date.now();
    if (!force && now - lastSave.current < SAVE_EVERY_MS) return;
    lastSave.current = now;
    recordPosition(key, {
      to: Math.round(el.currentTime * 1000),
      length: Math.round(el.duration * 1000),
      unit: "ms",
    });
    if (force) void flushProgress();
  };

  useEffect(() => {
    const el = audio.current;
    return () => {
      if (el && Number.isFinite(el.duration)) {
        recordPosition(key, {
          to: Math.round(el.currentTime * 1000),
          length: Math.round(el.duration * 1000),
          unit: "ms",
        });
        void flushProgress();
      }
    };
  }, [key]);

  if (!src) return null;
  return (
    <div
      className="library-audiobar"
      role="region"
      aria-label={t("library.audiobar", { title: book.title })}
      data-testid="library-audiobar"
    >
      <BookCover book={book} size="xs" />
      <div style={{ minWidth: 0, maxWidth: 220 }}>
        <div className="library-title" style={{ WebkitLineClamp: 1 }}>
          {book.title}
        </div>
        <div className="library-meta">{book.authors.join(", ")}</div>
      </div>
      <audio
        ref={audio}
        src={src}
        controls
        autoPlay
        preload="metadata"
        onLoadedMetadata={(e) => {
          const saved = loadProgress(key);
          if (saved && !saved.finished && saved.offset > 0)
            e.currentTarget.currentTime = saved.offset / 1000;
        }}
        onTimeUpdate={() => persist()}
        onPlay={() => setPlaying(true)}
        onPause={() => {
          setPlaying(false);
          persist(true);
        }}
        onSeeked={() => persist(true)}
        onEnded={() => persist(true)}
      />
      {/* Eigene, große Touchflächen (≥ 44 px); die eingebaute Leiste bleibt
          für Zeitanzeige und Springen per Schieber. */}
      <div className="library-audiobar__keys">
        <IconButton
          icon={<RotateCcw size={18} aria-hidden="true" />}
          label={t("library.back30")}
          onClick={() => skip(-30)}
        />
        <IconButton
          icon={
            playing ? (
              <Pause size={20} aria-hidden="true" />
            ) : (
              <Play size={20} aria-hidden="true" />
            )
          }
          label={playing ? t("library.pause") : t("library.play")}
          onClick={toggle}
          data-testid="library-audiobar-toggle"
        />
        <IconButton
          icon={<RotateCw size={18} aria-hidden="true" />}
          label={t("library.forward30")}
          onClick={() => skip(30)}
        />
        <IconButton
          icon={<X size={18} aria-hidden="true" />}
          label={t("library.audiobarClose")}
          onClick={onClose}
          data-testid="library-audiobar-close"
        />
      </div>
    </div>
  );
};
