import React from "react";
import { useTranslation } from "react-i18next";
import type { LibraryBook } from "@/bindings";
import { languageBadges } from "@/lib/library/editions";
import "./sprachfassungen.css";

/** Sprachkürzel einer Bibliothekskarte (sf-01): „DE · EN“, die zuletzt aktive hervorgehoben. */
export const LangBadges: React.FC<{ book: LibraryBook }> = ({ book }) => {
  const { t } = useTranslation();
  const badges = languageBadges(book);
  if (badges.length < 2) return null;
  const current = badges.find((b) => b.current);
  return (
    <span
      className="sf-langs"
      role="img"
      aria-label={t("library.sf.badges", {
        langs: badges.map((b) => b.label).join(", "),
        current: current?.label ?? "",
      })}
      data-testid={`sf-langs-${book.id}`}
    >
      {badges.map((b) => (
        <span key={b.id} className="sf-lg" data-on={b.current || undefined}>
          {b.code}
        </span>
      ))}
    </span>
  );
};
