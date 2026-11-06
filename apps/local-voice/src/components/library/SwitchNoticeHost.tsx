import React, { useEffect, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import {
  dismissNotice,
  getNotice,
  holdNotice,
  releaseNotice,
  subscribeNotice,
  type Notice,
} from "@/lib/library/notice";
import "./sprachfassungen.css";

const bold = (text: string): React.ReactNode[] =>
  text
    .split(/(\*\*[^*]+\*\*)/)
    .map((part, i) =>
      part.startsWith("**") ? <b key={i}>{part.slice(2, -2)}</b> : part,
    );

/**
 * Zeigt den aktuellen Hinweis (sf-06, sf-07, sf-11) über der Hörleiste bzw. im
 * Text-Leser. Die Zeit hält an, solange der Mauszeiger darüber steht oder der
 * Fokus darin liegt (Auflage des Prüfers). Die Aktionen sind echte Knöpfe.
 */
export const SwitchNoticeHost: React.FC<{ owner: Notice["owner"] }> = ({
  owner,
}) => {
  const { t } = useTranslation();
  const notice = useSyncExternalStore(subscribeNotice, getNotice, getNotice);
  const mine = notice && notice.owner === owner ? notice : null;
  // Wird die Leiste neu aufgebaut, hängt kein Zeiger und kein Fokus mehr daran.
  useEffect(
    () => () => {
      releaseNotice("hover");
      releaseNotice("focus");
    },
    [],
  );
  if (!mine) return null;
  return (
    <div
      className="sf-notice"
      data-tone={mine.tone}
      role={mine.tone === "alert" ? "alert" : "status"}
      data-testid="sf-notice"
      onMouseEnter={() => holdNotice("hover")}
      onMouseLeave={() => releaseNotice("hover")}
      onFocus={() => holdNotice("focus")}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null))
          releaseNotice("focus");
      }}
    >
      <span className="sf-notice__text">{bold(mine.text)}</span>
      <span className="sf-notice__actions">
        {mine.actions.map((a) => (
          <button
            key={a.id}
            type="button"
            className="sf-notice__action"
            data-testid={`sf-notice-${a.id}`}
            onClick={() => {
              dismissNotice(mine.id);
              void a.run();
            }}
          >
            {a.label}
          </button>
        ))}
        {mine.sticky && (
          <button
            type="button"
            className="sf-notice__action sf-notice__action--quiet"
            data-testid="sf-notice-close"
            onClick={() => dismissNotice(mine.id)}
          >
            {t("library.sf.notice.close")}
          </button>
        )}
      </span>
    </div>
  );
};
