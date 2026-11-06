import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  ArrowUpFromLine,
  Check,
  Crown,
  Download,
  ListTree,
  MoreVertical,
  RotateCcw,
  Trash2,
} from "lucide-react";
import i18n from "@/i18n";
import type { Edition, LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { ActionMenu, type ActionMenuItem } from "../ui/ActionMenu";
import {
  alignmentFor,
  audioKeyOf,
  deviceId,
  editionsOf,
  langName,
  langShort,
  primaryEdition,
  progressOfEdition,
  readingEdition,
  textKeyOf,
} from "@/lib/library/editions";
import {
  removeEdition,
  restoreEdition,
  setPrimaryEdition,
  splitEdition,
} from "@/lib/library/editionOps";
import { loadProgress, progressFraction } from "@/lib/library/progress";
import { formatDuration, totalDuration } from "@/lib/library/books";
import { UNDO_MS } from "@/lib/library/remove";
import "./sprachfassungen.css";

/** Rückholfrist im Papierkorb (wie im Kern). */
const TRASH_DAYS = 30;
const DAY_MS = 86_400_000;

/** „vor 2 Std.“, „vor 5 Min.“ in der Sprache der Oberfläche. */
export function relativeTime(ms: number, now = Date.now()): string {
  const rtf = new Intl.RelativeTimeFormat(i18n.language, { numeric: "auto" });
  const minutes = Math.round((ms - now) / 60_000);
  if (Math.abs(minutes) < 60) return rtf.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 48) return rtf.format(hours, "hour");
  return rtf.format(Math.round(hours / 24), "day");
}

/** Zeile „Zuletzt: Deutsch, Hörbuch, 29 % · am PC, vor 2 Std.“ für den Kopf der Buchseite. */
export const LastActive: React.FC<{ book: LibraryBook }> = ({ book }) => {
  const { t } = useTranslation();
  const e = readingEdition(book);
  if (!e || editionsOf(book).length < 2) return null;
  const p = progressOfEdition(book, e);
  const audio = !!p.audio && (p.audio.updated ?? 0) >= (p.text?.updated ?? 0);
  const rec = audio ? p.audio : p.text;
  const percent = Math.round(progressFraction(rec) * 100);
  const when = book.reading?.updated ?? p.lastUsed;
  const where =
    book.reading?.device && book.reading.device !== deviceId()
      ? t("library.sf.last.other")
      : t("library.sf.last.here");
  return (
    <div className="sf-meta" data-testid="sf-last">
      {t("library.sf.last.line", {
        lang: langName(e.lang) || langShort(e.lang),
        kind: t(audio ? "library.sf.kind.audio" : "library.sf.kind.text"),
        percent,
        where,
        when: when > 0 ? relativeTime(when) : "–",
      })}
    </div>
  );
};

type Action = { kind: "remove" | "split"; edition: Edition } | null;

interface Props {
  book: LibraryBook;
  onChanged(): Promise<void>;
  onAlign(pair: { a: string; b: string }): void;
  onExport?(): void;
}

/**
 * Abschnitt „Sprachfassungen“ der Buchseite (sf-04): Liste mit Text, Hörbuch
 * und Fortschritt je Fassung, „Zuletzt“, Menü am ⋮, getrennte Aktionen für
 * Abtrennen und Löschen (Papierkorb) und das Zurückholen gelöschter Fassungen.
 */
export const EditionsSection: React.FC<Props> = ({
  book,
  onChanged,
  onAlign,
  onExport,
}) => {
  const { t } = useTranslation();
  const list = editionsOf(book);
  const primary = primaryEdition(book);
  const reading = readingEdition(book);
  const [action, setAction] = useState<Action>(null);
  const [busy, setBusy] = useState(false);
  const [replacement, setReplacement] = useState<string>("");

  const now = Date.now();
  const restored = new Set(list.map((e) => e.id));
  const graves = (book.edition_tombstones ?? []).filter(
    (g) =>
      !restored.has(`${g.id}_r`) && g.removed_ms + TRASH_DAYS * DAY_MS > now,
  );

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    try {
      await task();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      await onChanged();
    }
  };

  const needsReplacement = (e: Edition) =>
    e.id === primary?.id || e.id === reading?.id;

  const begin = (kind: "remove" | "split", e: Edition) => {
    const others = list.filter((x) => x.id !== e.id);
    setReplacement(
      others.find((x) => x.id === primary?.id)?.id ?? others[0]?.id ?? "",
    );
    // Abtrennen einer Nebenfassung ist rückgängig zu machen (Zusammenlegen): sofort.
    if (kind === "split" && !needsReplacement(e)) {
      void doSplit(e, null);
      return;
    }
    setAction({ kind, edition: e });
  };

  const doSplit = (e: Edition, newPrimary: string | null) =>
    run(async () => {
      await splitEdition(book.id, e.id, newPrimary);
      toast(
        t("library.sf.split.done", {
          title: e.title || book.title,
          lang: langName(e.lang),
        }),
      );
    });

  const doRemove = (e: Edition, newPrimary: string | null) =>
    run(async () => {
      const after = await removeEdition(book.id, e.id, newPrimary);
      toast(t("library.sf.remove.done", { lang: langName(e.lang) }), {
        className: "library-undo-toast",
        description: t("library.sf.remove.doneHint"),
        duration: UNDO_MS,
        action: {
          label: t("library.remove.undo"),
          onClick: () =>
            void restoreEdition(after.id, e.id)
              .then(() => onChanged())
              .catch((err: unknown) =>
                toast.error(err instanceof Error ? err.message : String(err)),
              ),
        },
      });
    });

  const confirm = () => {
    if (!action) return;
    const { kind, edition } = action;
    const np = needsReplacement(edition) ? replacement || null : null;
    setAction(null);
    if (kind === "split") void doSplit(edition, np);
    else void doRemove(edition, np);
  };

  const menu = (e: Edition): ActionMenuItem[] => {
    const isPrimary = e.id === primary?.id;
    const other =
      list.find((x) => x.id !== e.id && x.id === primary?.id) ??
      list.find((x) => x.id !== e.id);
    const last = list.length < 2;
    return [
      {
        id: "primary",
        label: t("library.sf.menu.primary"),
        icon: Crown,
        disabled: isPrimary,
        testId: `sf-edition-primary-${e.id}`,
        onSelect: () =>
          void run(async () => {
            await setPrimaryEdition(book.id, e.id);
          }),
      },
      {
        id: "align",
        label: t("library.sf.menu.align"),
        icon: ListTree,
        disabled: !other,
        testId: `sf-edition-align-${e.id}`,
        onSelect: () => other && onAlign({ a: e.id, b: other.id }),
      },
      ...(onExport
        ? [
            {
              id: "export",
              label: t("library.sf.menu.export"),
              icon: Download,
              onSelect: onExport,
            } satisfies ActionMenuItem,
          ]
        : []),
      {
        id: "split",
        label: t("library.sf.menu.split"),
        icon: ArrowUpFromLine,
        disabled: last,
        dividerBefore: true,
        testId: `sf-edition-split-${e.id}`,
        onSelect: () => begin("split", e),
      },
      {
        id: "remove",
        label: t("library.sf.menu.remove"),
        description: t("library.sf.menu.removeHint"),
        icon: Trash2,
        danger: true,
        disabled: last,
        testId: `sf-edition-remove-${e.id}`,
        onSelect: () => begin("remove", e),
      },
    ];
  };

  const alignedLine = (() => {
    if (!primary || list.length < 2) return null;
    const other = list.find((x) => x.id !== primary.id);
    if (!other) return null;
    const { alignment, groups } = alignmentFor(book, primary.id, other.id);
    if (!alignment || !groups) return null;
    const all = groups.every((g) => g.confirmed);
    return t(
      all ? "library.sf.section.alignedAll" : "library.sf.section.alignedSome",
      { a: langName(primary.lang), b: langName(other.lang) },
    );
  })();

  const progress = (e: Edition): string => {
    const p = progressOfEdition(book, e);
    const rec =
      p.audio && (p.audio.updated ?? 0) >= (p.text?.updated ?? 0)
        ? p.audio
        : p.text;
    const pct = Math.round(progressFraction(rec) * 100);
    return pct > 0 ? `${pct} %` : "–";
  };

  const others = action ? list.filter((x) => x.id !== action.edition.id) : [];
  const hasProgress = (e: Edition) => {
    const k = textKeyOf(e);
    return (
      !!(k && loadProgress(k)) ||
      !!(e.audio && loadProgress(audioKeyOf(book, e)))
    );
  };

  if (list.length < 2 && !graves.length) return null;
  return (
    <section
      className="sf-editions"
      aria-labelledby="sf-editions-h"
      data-testid="sf-editions"
    >
      <h3 id="sf-editions-h" className="library-section__head">
        {t("library.sf.section.title")}
        {alignedLine && <small>{alignedLine}</small>}
      </h3>
      {list.length > 0 && (
        <table className="sf-table">
          <thead>
            <tr>
              <th scope="col">{t("library.sf.section.edition")}</th>
              <th scope="col">{t("library.sf.kind.text")}</th>
              <th scope="col">{t("library.sf.kind.audio")}</th>
              <th scope="col">{t("library.sf.section.progress")}</th>
              <th scope="col">
                <span className="sf-visually-hidden">
                  {t("library.sf.section.actions")}
                </span>
              </th>
            </tr>
          </thead>
          <tbody>
            {list.map((e) => {
              const dur = e.audio ? totalDuration(e.audio) : null;
              const label = langName(e.lang) || langShort(e.lang);
              return (
                <tr
                  key={e.id}
                  data-current={e.id === reading?.id || undefined}
                  data-testid={`sf-edition-${e.id}`}
                >
                  <th scope="row">
                    <b>{label}</b>
                    {e.id === primary?.id && (
                      <> · {t("library.sf.section.primary")}</>
                    )}
                    {e.id === reading?.id && list.length > 1 && (
                      <span className="sf-badge" data-tone="info">
                        {t("library.sf.section.last")}
                      </span>
                    )}
                    <span className="sf-meta sf-row__title">
                      {e.title || book.title}
                    </span>
                  </th>
                  <td>
                    {e.text ? (
                      <span role="img" aria-label={t("library.sf.section.has")}>
                        <Check size={14} aria-hidden="true" />
                      </span>
                    ) : (
                      <span aria-label={t("library.sf.section.hasNot")}>–</span>
                    )}
                  </td>
                  <td>
                    {e.audio ? (
                      <span role="img" aria-label={t("library.sf.section.has")}>
                        <Check size={14} aria-hidden="true" />{" "}
                        {dur ? formatDuration(dur) : ""}
                      </span>
                    ) : (
                      <span aria-label={t("library.sf.section.hasNot")}>–</span>
                    )}
                  </td>
                  <td>{hasProgress(e) ? progress(e) : "–"}</td>
                  <td className="library-touch sf-row__menu">
                    <ActionMenu
                      trigger={{
                        icon: MoreVertical,
                        label: t("library.sf.menu.for", { lang: label }),
                        description: t("library.sf.menu.for", { lang: label }),
                        testId: `sf-edition-menu-${e.id}`,
                        ghost: true,
                      }}
                      menuLabel={t("library.sf.menu.for", { lang: label })}
                      align="end"
                      widthClass="w-72"
                      items={menu(e)}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <p className="sf-meta sf-section__foot">{t("library.sf.section.foot")}</p>
      {graves.length > 0 && (
        <details className="sf-graves" data-testid="sf-graves">
          <summary>
            {t("library.sf.graves.title", { count: graves.length })}
          </summary>
          <ul>
            {graves.map((g, i) => (
              <li key={g.id}>
                <span>
                  {t("library.sf.graves.row", {
                    n: i + 1,
                    date: new Date(g.removed_ms).toLocaleDateString(
                      i18n.language,
                    ),
                    days: Math.max(
                      1,
                      Math.ceil(
                        (g.removed_ms + TRASH_DAYS * DAY_MS - now) / DAY_MS,
                      ),
                    ),
                  })}
                </span>
                <Button
                  icon={<RotateCcw size={14} aria-hidden="true" />}
                  disabled={busy}
                  onClick={() =>
                    void run(async () => {
                      await restoreEdition(book.id, g.id);
                      toast(t("library.sf.graves.done"));
                    })
                  }
                  data-testid={`sf-grave-restore-${g.id}`}
                >
                  {t("library.sf.graves.restore")}
                </Button>
              </li>
            ))}
          </ul>
        </details>
      )}
      <Dialog
        open={action !== null}
        onOpenChange={(o) => !o && setAction(null)}
        title={
          action?.kind === "remove"
            ? t("library.sf.remove.title", {
                lang: langName(action.edition.lang),
              })
            : t("library.sf.split.title", {
                lang: langName(action?.edition.lang),
              })
        }
        closeLabel={t("library.cancel")}
        footer={
          <>
            <Button onClick={() => setAction(null)}>
              {t("library.cancel")}
            </Button>
            <Button
              variant={action?.kind === "remove" ? "danger" : "primary"}
              disabled={
                !!action && needsReplacement(action.edition) && !replacement
              }
              onClick={confirm}
              data-testid="sf-edition-confirm"
            >
              {action?.kind === "remove"
                ? t("library.sf.remove.confirm")
                : t("library.sf.split.confirm")}
            </Button>
          </>
        }
      >
        {action && (
          <div className="sf-confirm" data-testid="sf-edition-dialog">
            <p>
              {action.kind === "remove"
                ? t("library.sf.remove.body", {
                    title: action.edition.title || book.title,
                  })
                : t("library.sf.split.body", {
                    title: action.edition.title || book.title,
                  })}
            </p>
            {needsReplacement(action.edition) && (
              <fieldset className="sf-offer__options">
                <legend>{t("library.sf.replace.legend")}</legend>
                {others.map((o) => (
                  <label
                    key={o.id}
                    className="sf-option"
                    data-selected={replacement === o.id || undefined}
                  >
                    <input
                      type="radio"
                      name="sf-replacement"
                      checked={replacement === o.id}
                      onChange={() => setReplacement(o.id)}
                      data-testid={`sf-replace-${o.id}`}
                    />
                    <span>
                      <b>{langName(o.lang) || langShort(o.lang)}</b>
                      <span className="sf-option__hint">{o.title}</span>
                    </span>
                  </label>
                ))}
              </fieldset>
            )}
          </div>
        )}
      </Dialog>
    </section>
  );
};
