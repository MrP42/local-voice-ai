import React from "react";

/**
 * Kleine Länderflagge als SVG. Windows stellt Flaggen-Emoji nur als zwei
 * Buchstaben dar -- deshalb eigene Zeichnungen für die Standorte, die das
 * Regelwerk kennt. Unbekannte Kürzel erscheinen als Text.
 */
const W = 16;
const H = 11;

const stripesV = (colors: string[]) =>
  colors.map((c, i) => (
    <rect
      key={i}
      x={(W / colors.length) * i}
      y={0}
      width={W / colors.length}
      height={H}
      fill={c}
    />
  ));

const stripesH = (colors: string[]) =>
  colors.map((c, i) => (
    <rect
      key={i}
      x={0}
      y={(H / colors.length) * i}
      width={W}
      height={H / colors.length}
      fill={c}
    />
  ));

const FLAGS: Record<string, React.ReactNode> = {
  DE: stripesH(["#000", "#dd0000", "#ffce00"]),
  FR: stripesV(["#002395", "#fff", "#ed2939"]),
  IT: stripesV(["#009246", "#fff", "#ce2b37"]),
  IE: stripesV(["#169b62", "#fff", "#ff883e"]),
  SE: (
    <>
      <rect width={W} height={H} fill="#006aa7" />
      <rect x={5} y={0} width={2} height={H} fill="#fecc00" />
      <rect x={0} y={4.5} width={W} height={2} fill="#fecc00" />
    </>
  ),
  US: (
    <>
      {Array.from({ length: 7 }, (_, i) => (
        <rect
          key={i}
          x={0}
          y={i * (H / 6.5)}
          width={W}
          height={H / 13}
          fill="#b22234"
        />
      ))}
      <rect width={7} height={6} fill="#3c3b6e" />
    </>
  ),
  EU: (
    <>
      <rect width={W} height={H} fill="#003399" />
      {Array.from({ length: 12 }, (_, i) => {
        const a = (i / 12) * Math.PI * 2;
        return (
          <circle
            key={i}
            cx={8 + Math.sin(a) * 3.4}
            cy={5.5 - Math.cos(a) * 3.4}
            r={0.6}
            fill="#ffcc00"
          />
        );
      })}
    </>
  ),
};

export const Flag: React.FC<{ code: string; title?: string }> = ({
  code,
  title,
}) => {
  const drawing = FLAGS[code.toUpperCase()];
  if (!drawing) {
    return (
      <span
        className="text-[10px] font-semibold text-text/60"
        title={title}
        data-flag={code}
      >
        {code}
      </span>
    );
  }
  return (
    <svg
      width={W}
      height={H}
      viewBox={`0 0 ${W} ${H}`}
      role="img"
      aria-label={title ?? code}
      className="inline-block shrink-0 rounded-[2px] ring-1 ring-black/10"
      data-flag={code}
    >
      {title && <title>{title}</title>}
      {drawing}
    </svg>
  );
};

export default Flag;
