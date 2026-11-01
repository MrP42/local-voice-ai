import React from "react";
import { useTranslation } from "react-i18next";
import { Check, Trash2, X } from "lucide-react";
import { Button } from "../ui/Button";
import {
  statusOf,
  type LearnedPair,
  type LearnedStatus,
} from "./speechLearning";

const STATUS_STYLE: Record<LearnedStatus, string> = {
  beobachtet: "bg-mid-gray/20 text-text/70",
  vorschlag: "bg-amber-500/20 text-amber-800 dark:text-amber-300",
  uebernommen: "bg-green-500/20 text-green-800 dark:text-green-300",
  abgelehnt: "bg-mid-gray/20 text-text/60 line-through",
};

/** Eine Zeile „gehört → korrigiert“ mit Zahl, Status und Knöpfen. */
export const PairLine: React.FC<{
  pair: LearnedPair;
  onSet: (status: LearnedStatus) => void;
  onDelete?: () => void;
}> = ({ pair, onSet, onDelete }) => {
  const { t } = useTranslation();
  const names = { gehoert: pair.gehoert, korrigiert: pair.korrigiert };
  return (
    <li
      data-testid="learned-pair"
      className="rounded-lg border border-mid-gray/25 px-3 py-2 flex flex-col gap-1.5"
    >
      <div className="text-[13px] break-words">
        <span className="text-text/70">{pair.gehoert}</span>
        <span aria-hidden="true"> → </span>
        <span className="sr-only">{t("voiceCall.learning.becomes")}</span>
        <span className="font-semibold">{pair.korrigiert}</span>
      </div>
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-xs text-text/60">
          {t("voiceCall.learning.times", { count: pair.anzahl })}
        </span>
        <span
          className={`inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium ${STATUS_STYLE[statusOf(pair)]}`}
        >
          {t(`voiceCall.learning.status.${statusOf(pair)}`)}
        </span>
        <span className="ms-auto flex flex-wrap items-center gap-1">
          {statusOf(pair) !== "uebernommen" && (
            <Button
              size="sm"
              type="button"
              icon={<Check className="w-3.5 h-3.5" aria-hidden="true" />}
              aria-label={t("voiceCall.learning.adoptLabel", names)}
              onClick={() => onSet("uebernommen")}
            >
              {t("voiceCall.learning.adopt")}
            </Button>
          )}
          {statusOf(pair) !== "abgelehnt" && (
            <Button
              size="sm"
              type="button"
              icon={<X className="w-3.5 h-3.5" aria-hidden="true" />}
              aria-label={t("voiceCall.learning.rejectLabel", names)}
              onClick={() => onSet("abgelehnt")}
            >
              {t("voiceCall.learning.reject")}
            </Button>
          )}
          {onDelete && (
            <Button
              size="sm"
              variant="ghost"
              type="button"
              icon={<Trash2 className="w-3.5 h-3.5" aria-hidden="true" />}
              aria-label={t("voiceCall.learning.deleteLabel", names)}
              title={t("voiceCall.learning.delete")}
              onClick={onDelete}
            />
          )}
        </span>
      </div>
    </li>
  );
};
