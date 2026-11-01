import React, { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
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
        onPause={() => persist(true)}
        onSeeked={() => persist(true)}
        onEnded={() => persist(true)}
      />
      <IconButton
        icon={<X size={16} aria-hidden="true" />}
        label={t("library.audiobarClose")}
        onClick={onClose}
      />
    </div>
  );
};
