import React, { useState } from "react";
import type { LibraryBook } from "@/bindings";
import { assetUrl } from "@/lib/library/books";

/** Einfacher, stabiler Hash: dasselbe Buch bekommt immer dasselbe Cover. */
function hash(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

const PATTERNS = ["rings", "lines", "waves", "diamond", "frame"] as const;

function Pattern({ kind }: { kind: (typeof PATTERNS)[number] }) {
  const common = {
    className: "book-cover__pattern",
    "aria-hidden": true,
    viewBox: "0 0 100 150",
  } as const;
  switch (kind) {
    case "rings":
      return (
        <svg {...common} preserveAspectRatio="xMidYMid slice">
          <g fill="none" stroke="#fff" strokeWidth=".7">
            {[10, 20, 30, 40, 50, 60].map((r) => (
              <circle key={r} cx="78" cy="38" r={r} />
            ))}
          </g>
        </svg>
      );
    case "lines":
      return (
        <svg {...common} preserveAspectRatio="none">
          <g stroke="#fff" strokeWidth=".5">
            {Array.from({ length: 22 }, (_, i) => (
              <line key={i} x1="0" x2="100" y1={i * 7} y2={i * 7 + 30} />
            ))}
          </g>
        </svg>
      );
    case "waves":
      return (
        <svg {...common} preserveAspectRatio="none">
          <g fill="none" stroke="#fff" strokeWidth=".8">
            {Array.from({ length: 9 }, (_, i) => (
              <path
                key={i}
                d={`M0 ${70 + i * 9} q12 -8 25 0 t25 0 t25 0 t25 0`}
              />
            ))}
          </g>
        </svg>
      );
    case "diamond":
      return (
        <svg {...common} preserveAspectRatio="xMidYMid slice">
          <g fill="none" stroke="#fff" strokeWidth=".6">
            {[14, 24, 34, 44].map((r) => (
              <path
                key={r}
                d={`M50 ${75 - r} L${50 + r} 75 L50 ${75 + r} L${50 - r} 75Z`}
              />
            ))}
          </g>
        </svg>
      );
    default:
      return (
        <svg {...common} preserveAspectRatio="none">
          <rect x="6" y="6" width="88" height="138" fill="none" stroke="#fff" strokeWidth="1" />
          <rect x="10" y="10" width="80" height="130" fill="none" stroke="#fff" strokeWidth=".4" />
        </svg>
      );
  }
}

/**
 * Cover eines Buchs: das abgelegte Bild (Open Library, EPUB, eigene Datei),
 * sonst ein generiertes aus Titel und Autor – Farbe und Muster hängen am
 * Buch, damit es in Raster und Detail gleich aussieht. `size` steuert die
 * Schriftgrößen des generierten Covers.
 */
export const BookCover: React.FC<{
  book: LibraryBook;
  size?: "xs" | "sm" | "md" | "lg";
  className?: string;
}> = ({ book, size = "md", className = "" }) => {
  const [failed, setFailed] = useState<string | null>(null);
  const src = assetUrl(book.cover_path);
  const author = book.authors.join(", ");
  if (src && failed !== src) {
    return (
      <div className={`book-cover book-cover--${size} ${className}`}>
        <img
          src={src}
          alt=""
          className="book-cover__img"
          loading="lazy"
          draggable={false}
          onError={() => setFailed(src)}
        />
      </div>
    );
  }
  const h = hash(`${book.title}|${author}`);
  const hue = h % 360;
  const sat = 25 + (h % 30);
  const light = 22 + ((h >> 8) % 14);
  const pattern = PATTERNS[(h >> 4) % PATTERNS.length];
  return (
    <div
      className={`book-cover book-cover--generated book-cover--${size} ${className}`}
      style={{
        background: `linear-gradient(160deg, hsl(${hue} ${sat}% ${light + 12}%), hsl(${hue} ${sat}% ${light}%) 55%, hsl(${(hue + 20) % 360} ${sat}% ${Math.max(light - 8, 8)}%))`,
      }}
      aria-hidden="true"
    >
      <Pattern kind={pattern} />
      <span className="book-cover__author">{author}</span>
      <span className="book-cover__title" lang="de">{book.title}</span>
    </div>
  );
};
