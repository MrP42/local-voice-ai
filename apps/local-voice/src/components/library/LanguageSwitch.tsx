import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Globe } from "lucide-react";
import type { LibraryBook } from "@/bindings";
import {
  alignmentFor,
  audioKeyOf,
  editionById,
  editionsOf,
  hasAudio,
  langName,
  langShort,
  textKeyOf,
} from "@/lib/library/editions";
import { loadProgress, progressFraction } from "@/lib/library/progress";
import type { SwitchMedium } from "@/lib/library/switching";
import "./sprachfassungen.css";

interface Props {
  book: LibraryBook;
  /** Fassung, die gerade läuft. */
  currentId: string;
  medium: SwitchMedium;
  onPick(editionId: string): void;
  /** „Kapitel zuordnen …“ im Menü anbieten. */
  onAlign?(): void;
  /** Fassungen, deren Hörbuch gerade geladen wird (Anzeige als Text). */
  loading?: ReadonlySet<string>;
  /** Fokus beim Einhängen auf den Knopf setzen (nach einem Wechsel). */
  focusOnMount?: boolean;
  /** Dropup (Hörleiste, Standard) oder Dropdown (Kopfleiste des Lesers). */
  placement?: "up" | "down";
}

/**
 * Sprachknopf mit Dropup (sf-05): ein Knopf mit Sprachkürzel, darüber die
 * Fassungen als `menuitemradio`. Pfeiltasten wählen, Eingabe wechselt, Esc
 * schließt und gibt den Fokus an den Knopf zurück. Fassungen ohne Hörbuch bzw.
 * Text auf diesem Gerät sind beschriftet statt versteckt.
 */
export const LanguageSwitch: React.FC<Props> = ({
  book,
  currentId,
  medium,
  onPick,
  onAlign,
  loading,
  focusOnMount = false,
  placement = "up",
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const editions = editionsOf(book);
  const current = editionById(book, currentId);

  useEffect(() => {
    if (focusOnMount) button.current?.focus();
  }, [focusOnMount]);

  const items = () =>
    Array.from(
      menu.current?.querySelectorAll<HTMLButtonElement>(
        '[role="menuitemradio"]:not(:disabled), [role="menuitem"]:not(:disabled)',
      ) ?? [],
    );

  // Beim Öffnen: Fokus auf die gewählte Sprache.
  useEffect(() => {
    if (!open) return;
    const list = items();
    (
      list.find((el) => el.getAttribute("aria-checked") === "true") ?? list[0]
    )?.focus();
  }, [open]);

  // Klick daneben schließt (der Fokus bleibt dort, wo geklickt wurde).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (menu.current?.contains(target) || button.current?.contains(target))
        return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) return;
    const list = items();
    const at = list.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => {
      e.preventDefault();
      list[(i + list.length) % list.length]?.focus();
    };
    switch (e.key) {
      case "Escape":
        e.preventDefault();
        e.stopPropagation();
        close(true);
        return;
      case "Tab":
        close(true);
        return;
      case "ArrowDown":
        return go(at + 1);
      case "ArrowUp":
        return go(at - 1);
      case "Home":
        return go(0);
      case "End":
        return go(list.length - 1);
    }
  };

  const detail = (id: string) => {
    const e = editionById(book, id);
    if (!e) return "";
    const has = medium === "audio" ? hasAudio(book, e) : !!e.text;
    const kind = t(
      medium === "audio" ? "library.sf.switch.audio" : "library.sf.switch.text",
    );
    if (!has) {
      if (medium === "audio" && e.audio)
        return loading?.has(id)
          ? t("library.sf.switch.loading")
          : t("library.sf.switch.notLocal");
      return t(
        medium === "audio"
          ? "library.sf.switch.noAudio"
          : "library.sf.switch.noText",
      );
    }
    const key = medium === "audio" ? audioKeyOf(book, e) : textKeyOf(e);
    const percent = Math.round(
      progressFraction(key ? loadProgress(key) : null) * 100,
    );
    const narrator =
      medium === "audio" && e.audio?.narrator
        ? t("library.sf.switch.narrator", { name: e.audio.narrator })
        : null;
    return [kind, narrator, percent > 0 ? `${percent} %` : null]
      .filter(Boolean)
      .join(" · ");
  };

  const others = editions.filter((e) => e.id !== currentId);
  const aligned = others
    .map((o) => ({ o, groups: alignmentFor(book, currentId, o.id).groups }))
    .find((x) => x.groups?.length && x.groups.every((g) => g.confirmed));

  if (editions.length < 2) return null;
  return (
    <div className="sf-switch" data-placement={placement} onKeyDown={onKeyDown}>
      <button
        ref={button}
        type="button"
        className="sf-switch__button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={t("library.sf.switch.button", {
          lang: langName(current?.lang) || langShort(current?.lang),
        })}
        data-testid="sf-switch-button"
        onClick={() => setOpen((o) => !o)}
      >
        <Globe size={16} aria-hidden="true" />
        <span>{langShort(current?.lang)}</span>
        <span aria-hidden="true">{open ? "▴" : "▾"}</span>
      </button>
      {open && (
        <div
          ref={menu}
          id={menuId}
          role="menu"
          aria-label={t("library.sf.switch.menu")}
          className="sf-switch__menu"
          data-testid="sf-switch-menu"
        >
          <div className="sf-switch__head" aria-hidden="true">
            {t("library.sf.switch.menu")}
          </div>
          {editions.map((e) => {
            const checked = e.id === currentId;
            const usable =
              medium === "audio" ? hasAudio(book, e) || !!e.audio : !!e.text;
            return (
              <button
                key={e.id}
                type="button"
                role="menuitemradio"
                aria-checked={checked}
                disabled={!usable}
                className="sf-switch__item"
                data-testid={`sf-switch-item-${e.id}`}
                onClick={() => {
                  close(true);
                  if (!checked) onPick(e.id);
                }}
              >
                <span className="sf-switch__check" aria-hidden="true">
                  {checked && <Check size={14} />}
                </span>
                <span className="sf-switch__label">
                  <b>{langName(e.lang) || langShort(e.lang)}</b>
                  <span className="sf-switch__detail">{detail(e.id)}</span>
                </span>
              </button>
            );
          })}
          <div className="sf-switch__note" data-testid="sf-switch-note">
            {aligned
              ? t("library.sf.switch.alignedNote", {
                  a: langName(current?.lang),
                  b: langName(aligned.o.lang),
                })
              : t("library.sf.switch.openNote")}
          </div>
          {onAlign && (
            <button
              type="button"
              role="menuitem"
              className="sf-switch__item sf-switch__item--plain"
              data-testid="sf-switch-align"
              onClick={() => {
                close(false);
                onAlign();
              }}
            >
              <span className="sf-switch__check" aria-hidden="true" />
              <span className="sf-switch__label">
                {t("library.sf.switch.align")}
              </span>
            </button>
          )}
        </div>
      )}
    </div>
  );
};
