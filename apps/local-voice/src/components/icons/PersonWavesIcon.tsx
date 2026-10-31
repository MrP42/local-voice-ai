import React from "react";

/**
 * Rollensymbol „Spracherkennung“: Person mit Schallwellen. Lucide hat kein
 * passendes Symbol; Linienstärke und Raster folgen den Lucide-Icons, damit
 * es in der Fußleiste neben ihnen nicht auffällt.
 */
export const PersonWavesIcon: React.FC<{
  size?: number;
  className?: string;
}> = ({ size = 14, className }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth={2}
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
    focusable="false"
    className={className}
  >
    <circle cx="9" cy="8" r="3.5" />
    <path d="M2.5 20c0-3.6 2.9-6 6.5-6s6.5 2.4 6.5 6" />
    <path d="M17.5 6.5a4 4 0 0 1 0 7" />
    <path d="M20.5 4a7.5 7.5 0 0 1 0 12" />
  </svg>
);
