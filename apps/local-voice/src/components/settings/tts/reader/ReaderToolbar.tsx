import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { BookOpen, Check, Eye, FileText, PenLine, Type } from "lucide-react";

export const FONT_SIZES = [
  { value: "16", key: "fontSmall" },
  { value: "18", key: "fontMedium" },
  { value: "21", key: "fontLarge" },
] as const;

interface ReaderToolbarProps {
  mode: "read" | "edit";
  /** Lesen nur mit Text möglich. */
  canRead: boolean;
  onMode(mode: "read" | "edit"): void;
  twoPages: boolean;
  /** Zwei Seiten gewählt, aber die Fläche ist zu schmal. */
  twoPagesTooNarrow: boolean;
  onTwoPages(two: boolean): void;
  fontSize: string;
  onFontSize(size: string): void;
  showSpeakers: boolean;
  onShowSpeakers(show: boolean): void;
  follow: boolean;
  onFollow(follow: boolean): void;
  /** Lesezeichen-Knopf (nur in der Leseansicht). */
  bookmarks?: React.ReactNode;
}

const SEG =
  "inline-flex items-center gap-0.5 rounded-md border border-mid-gray/30 bg-mid-gray/10 p-0.5";
const SEG_BTN =
  "flex h-7 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded px-2 text-xs text-text/70 hover:text-text disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";
const SEG_ON = "bg-background font-semibold text-text shadow-sm";

/** Kopfleiste der Buchansicht: Lesen/Bearbeiten, Seitenansicht, Schrift. */
export const ReaderToolbar: React.FC<ReaderToolbarProps> = ({
  mode,
  canRead,
  onMode,
  twoPages,
  twoPagesTooNarrow,
  onTwoPages,
  fontSize,
  onFontSize,
  showSpeakers,
  onShowSpeakers,
  follow,
  onFollow,
  bookmarks,
}) => {
  const { t } = useTranslation();
  const [menu, setMenu] = useState(false);
  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setMenu(false);
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, [menu]);
  const twoTitle = twoPagesTooNarrow
    ? t("tts.reader.twoPagesTooNarrow")
    : t("tts.reader.twoPages");
  return (
    <div className="flex shrink-0 items-center gap-2" data-testid="reader-toolbar">
      <div className={SEG} role="group" aria-label={t("tts.reader.modeLabel")}>
        <button
          type="button"
          className={`${SEG_BTN} ${mode === "read" ? SEG_ON : ""}`}
          aria-pressed={mode === "read"}
          disabled={!canRead}
          onClick={() => onMode("read")}
          data-testid="reader-mode-read"
        >
          <Eye width={14} height={14} aria-hidden="true" />
          {t("tts.reader.modeRead")}
        </button>
        <button
          type="button"
          className={`${SEG_BTN} ${mode === "edit" ? SEG_ON : ""}`}
          aria-pressed={mode === "edit"}
          onClick={() => onMode("edit")}
          data-testid="reader-mode-edit"
        >
          <PenLine width={14} height={14} aria-hidden="true" />
          {t("tts.reader.modeEdit")}
        </button>
      </div>
      {mode === "read" && (
        <>
          <div className={SEG} role="group" aria-label={t("tts.reader.display")}>
            <button
              type="button"
              className={`${SEG_BTN} ${!twoPages ? SEG_ON : ""}`}
              aria-pressed={!twoPages}
              aria-label={t("tts.reader.onePage")}
              title={t("tts.reader.onePage")}
              onClick={() => onTwoPages(false)}
              data-testid="reader-one-page"
            >
              <FileText width={14} height={14} aria-hidden="true" />
            </button>
            <button
              type="button"
              className={`${SEG_BTN} ${twoPages ? SEG_ON : ""}`}
              aria-pressed={twoPages}
              aria-label={twoTitle}
              title={twoTitle}
              onClick={() => onTwoPages(true)}
              data-testid="reader-two-pages"
            >
              <BookOpen width={14} height={14} aria-hidden="true" />
            </button>
          </div>
          {bookmarks}
          <div className="relative">
            <button
              type="button"
              className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
              aria-haspopup="menu"
              aria-expanded={menu}
              aria-label={t("tts.reader.display")}
              title={t("tts.reader.display")}
              onClick={() => setMenu((m) => !m)}
              data-testid="reader-display"
            >
              <Type width={16} height={16} aria-hidden="true" />
            </button>
            {menu && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setMenu(false)} />
                <div
                  role="menu"
                  aria-label={t("tts.reader.display")}
                  className="absolute right-0 top-full z-40 mt-1 w-60 rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
                >
                  {FONT_SIZES.map((f) => (
                    <button
                      key={f.value}
                      type="button"
                      role="menuitemradio"
                      aria-checked={fontSize === f.value}
                      onClick={() => onFontSize(f.value)}
                      className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15"
                    >
                      <Check
                        width={14}
                        height={14}
                        aria-hidden="true"
                        className={fontSize === f.value ? "" : "invisible"}
                      />
                      {t(`tts.reader.${f.key}`)}
                    </button>
                  ))}
                  <hr className="my-1 border-mid-gray/20" />
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={showSpeakers}
                    onClick={() => onShowSpeakers(!showSpeakers)}
                    className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15"
                  >
                    <Check width={14} height={14} aria-hidden="true" className={showSpeakers ? "" : "invisible"} />
                    {t("tts.reader.showSpeakers")}
                  </button>
                  <button
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={follow}
                    onClick={() => onFollow(!follow)}
                    className="flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15"
                  >
                    <Check width={14} height={14} aria-hidden="true" className={follow ? "" : "invisible"} />
                    {t("tts.reader.follow")}
                  </button>
                </div>
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
};
