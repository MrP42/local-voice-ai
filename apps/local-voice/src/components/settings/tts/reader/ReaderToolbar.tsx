import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  BookOpen,
  Check,
  Eye,
  FileText,
  Maximize2,
  Minimize2,
  PanelRightOpen,
  PenLine,
  Sun,
  Type,
} from "lucide-react";

export const FONT_SIZES = [
  { value: "16", key: "fontSmall" },
  { value: "18", key: "fontMedium" },
  { value: "21", key: "fontLarge" },
] as const;

/** Leseschriften (lva#148, Kindle-Vorbild): nur Systemschriften. */
export const READER_FONTS = [
  { value: "book", key: "fontBook" },
  { value: "sans", key: "fontSans" },
  { value: "wide", key: "fontWide" },
] as const;
export type ReaderFont = (typeof READER_FONTS)[number]["value"];

/** Farbthemen der Buchseite (Kindle: Weiß, Sepia, Grün, Schwarz). */
export const READER_THEMES = [
  { value: "auto", key: "themeAuto", swatch: "" },
  { value: "white", key: "themeWhite", swatch: "#ffffff" },
  { value: "sepia", key: "themeSepia", swatch: "#f4ecd8" },
  { value: "green", key: "themeGreen", swatch: "#d5ead2" },
  { value: "black", key: "themeBlack", swatch: "#000000" },
] as const;
export type ReaderTheme = (typeof READER_THEMES)[number]["value"];

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
  fontFamily: ReaderFont;
  onFontFamily(font: ReaderFont): void;
  theme: ReaderTheme;
  onTheme(theme: ReaderTheme): void;
  /** Bildschirm wach halten (in beiden Ansichten). */
  keepAwake: boolean;
  onKeepAwake(on: boolean): void;
  /** Vollbild der Leseansicht. */
  fullscreen: boolean;
  onFullscreen(): void;
  /** ⓘ-Knopf mit den Schreibregeln (in beiden Ansichten). */
  info?: React.ReactNode;
  /** Rechte Spalte ist ausgeblendet: beschrifteter Knopf zum Einblenden. */
  onShowColumn?(): void;
}

const ICON_BTN =
  "flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary";
const MENU_ITEM =
  "flex w-full cursor-pointer items-center gap-2 px-3 py-1.5 text-start text-sm hover:bg-mid-gray/15";
const MENU_HEAD = "px-3 pb-0.5 pt-1.5 text-xs font-semibold text-text/70";

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
  fontFamily,
  onFontFamily,
  theme,
  onTheme,
  keepAwake,
  onKeepAwake,
  fullscreen,
  onFullscreen,
  info,
  onShowColumn,
}) => {
  const { t } = useTranslation();
  const [menu, setMenu] = useState(false);
  // Das Menü darf nie unter den Fensterrand ragen (390 px): Höhe auf den
  // Platz unter dem Knopf begrenzen, der Rest scrollt im Menü.
  const [menuMax, setMenuMax] = useState<number | null>(null);
  const displayBtnRef = useRef<HTMLButtonElement>(null);
  useLayoutEffect(() => {
    if (!menu) return;
    const fit = () => {
      const btn = displayBtnRef.current;
      if (!btn) return;
      // Unterkante: Fenster oder ein scrollender Vorfahr (die Lesespalte
      // schneidet sonst ab), je nachdem, was höher liegt.
      let bottom = window.innerHeight;
      for (let el = btn.parentElement; el; el = el.parentElement) {
        const o = getComputedStyle(el).overflowY;
        if (o !== "visible") bottom = Math.min(bottom, el.getBoundingClientRect().bottom);
      }
      const r = btn.getBoundingClientRect();
      setMenuMax(Math.max(160, Math.floor(bottom - r.bottom - 12)));
    };
    fit();
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  }, [menu]);
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
  const awakeLabel = keepAwake
    ? t("tts.reader.keepAwakeOn")
    : t("tts.reader.keepAwake");
  const awake = (
    <button
      type="button"
      className={`${ICON_BTN} ${keepAwake ? "bg-logo-primary/15 text-text" : ""}`}
      aria-pressed={keepAwake}
      aria-label={t("tts.reader.keepAwake")}
      title={awakeLabel}
      onClick={() => onKeepAwake(!keepAwake)}
      data-testid="reader-keep-awake"
    >
      <Sun width={16} height={16} aria-hidden="true" />
    </button>
  );
  const twoTitle = twoPagesTooNarrow
    ? t("tts.reader.twoPagesTooNarrow")
    : t("tts.reader.twoPages");
  return (
    <div className="flex max-w-full flex-wrap items-center justify-end gap-x-2 gap-y-1" data-testid="reader-toolbar">
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
      {mode === "edit" && awake}
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
          {awake}
          <div className="relative">
            <button
              type="button"
              className={ICON_BTN}
              aria-haspopup="menu"
              aria-expanded={menu}
              aria-label={t("tts.reader.display")}
              title={t("tts.reader.display")}
              ref={displayBtnRef}
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
                  className="absolute right-0 top-full z-40 mt-1 w-60 overflow-y-auto overscroll-contain rounded-lg border border-mid-gray/40 bg-background py-1 shadow-lg"
                  style={{ maxHeight: menuMax ?? undefined }}
                  data-testid="reader-display-menu"
                >
                  <p className={MENU_HEAD} aria-hidden="true">
                    {t("tts.reader.sizeHead")}
                  </p>
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
                  <p className={MENU_HEAD} aria-hidden="true">
                    {t("tts.reader.fontHead")}
                  </p>
                  {READER_FONTS.map((f) => (
                    <button
                      key={f.value}
                      type="button"
                      role="menuitemradio"
                      aria-checked={fontFamily === f.value}
                      onClick={() => onFontFamily(f.value)}
                      className={MENU_ITEM}
                      data-testid={`reader-font-${f.value}`}
                    >
                      <Check
                        width={14}
                        height={14}
                        aria-hidden="true"
                        className={fontFamily === f.value ? "" : "invisible"}
                      />
                      <span className={`reader-fontsample reader-fontsample--${f.value}`}>
                        {t(`tts.reader.${f.key}`)}
                      </span>
                    </button>
                  ))}
                  <hr className="my-1 border-mid-gray/20" />
                  <p className={MENU_HEAD} aria-hidden="true">
                    {t("tts.reader.themeHead")}
                  </p>
                  {READER_THEMES.map((th) => (
                    <button
                      key={th.value}
                      type="button"
                      role="menuitemradio"
                      aria-checked={theme === th.value}
                      onClick={() => onTheme(th.value)}
                      className={MENU_ITEM}
                      data-testid={`reader-theme-${th.value}`}
                    >
                      <Check
                        width={14}
                        height={14}
                        aria-hidden="true"
                        className={theme === th.value ? "" : "invisible"}
                      />
                      <span
                        aria-hidden="true"
                        className="h-4 w-4 shrink-0 rounded-full border border-mid-gray/50"
                        style={
                          th.swatch
                            ? { background: th.swatch }
                            : {
                                background:
                                  "linear-gradient(135deg, #fff 50%, #1f1f1f 50%)",
                              }
                        }
                      />
                      {t(`tts.reader.${th.key}`)}
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
          <button
            type="button"
            className={ICON_BTN}
            aria-pressed={fullscreen}
            aria-label={
              fullscreen ? t("tts.reader.fullscreenExit") : t("tts.reader.fullscreen")
            }
            title={fullscreen ? t("tts.reader.fullscreenExit") : t("tts.reader.fullscreen")}
            onClick={onFullscreen}
            data-testid="reader-fullscreen"
          >
            {fullscreen ? (
              <Minimize2 width={16} height={16} aria-hidden="true" />
            ) : (
              <Maximize2 width={16} height={16} aria-hidden="true" />
            )}
          </button>
        </>
      )}
      {info}
      {onShowColumn && (
        <button
          type="button"
          className="flex h-8 cursor-pointer items-center gap-1.5 whitespace-nowrap rounded-md border border-mid-gray/30 px-2 text-xs text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
          title={t("tts.layout.showColumnHint")}
          onClick={onShowColumn}
          data-testid="right-column-show"
        >
          <PanelRightOpen width={14} height={14} aria-hidden="true" />
          {t("tts.layout.showColumn")}
        </button>
      )}
    </div>
  );
};
