import React, { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { MessageSquare, Users } from "lucide-react";
import type { Participant } from "@/bindings";
import { initials } from "@/lib/meetingPeople";
import { Button } from "../../../ui/Button";

const PANEL_WIDTH = 288;
const MARGIN = 8;

export interface PersonRef {
  id: string;
  name: string;
}

interface PersonPopoverProps {
  participant: Participant;
  /** Liste auf Besprechungen mit dieser Person eingrenzen. */
  onFilter: (person: PersonRef) => void;
  /** Chat ueber alle Besprechungen mit dieser Person. */
  onAsk: (person: PersonRef) => void;
  /** "Personen verwalten ..." */
  onManage: () => void;
  /** Nur der Avatar (Initialen) in der Chipzeile; der Name steht im Tooltip. */
  compact?: boolean;
}

/**
 * Eine teilnehmende Person als Chip in der Kopfzeile der Besprechung (M5-P5d).
 * Ein Klick oeffnet ein Popover: E-Mail, Firma, "N Besprechungen mit Anna"
 * (filtert die Liste), "Fragen" (Chat ueber diese Person) und "Personen
 * verwalten ...".
 */
export const PersonPopover: React.FC<PersonPopoverProps> = ({
  participant,
  onFilter,
  onAsk,
  onManage,
  compact = false,
}) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const person: PersonRef = {
    id: participant.human_id,
    name: participant.name,
  };

  const place = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) return;
    const rect = trigger.getBoundingClientRect();
    const height = panelRef.current?.offsetHeight ?? 0;
    const left = Math.max(
      MARGIN,
      Math.min(rect.left, window.innerWidth - PANEL_WIDTH - MARGIN),
    );
    const below = rect.bottom + 4;
    const top =
      height > 0 && below + height > window.innerHeight - MARGIN
        ? Math.max(MARGIN, rect.top - height - 4)
        : below;
    setPos({ left, top });
  }, []);

  const close = useCallback(() => setOpen(false), []);

  useEffect(() => {
    if (!open) return;
    place();
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open, place]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        panelRef.current?.contains(target) ||
        triggerRef.current?.contains(target)
      )
        return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        close();
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, close]);

  const run = (action: () => void) => {
    close();
    action();
  };

  const count = participant.meeting_count;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={
          compact
            ? `${participant.name} · ${t(
                `meetings.people.role.${participant.role}`,
                { defaultValue: participant.role },
              )}`
            : t(`meetings.people.role.${participant.role}`, {
                defaultValue: participant.role,
              })
        }
        data-testid="participant-chip"
        data-human-id={participant.human_id}
        data-role={participant.role}
        className={`inline-flex items-center rounded-full border text-xs cursor-pointer transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary/60 ${
          compact ? "gap-0 p-0.5" : "gap-1.5 px-2 py-0.5"
        } ${
          open
            ? "bg-logo-primary/20 border-logo-primary text-text"
            : "border-mid-gray/40 text-text/80 hover:bg-mid-gray/15 hover:text-text"
        }`}
      >
        <span
          aria-hidden="true"
          className={`flex items-center justify-center rounded-full bg-mid-gray/25 font-semibold ${
            compact ? "h-5 w-5 text-[10px]" : "h-4 w-4 text-[9px]"
          }`}
        >
          {initials(participant.name)}
        </span>
        <span className={compact ? "sr-only" : "max-w-[12rem] truncate"}>
          {participant.name}
        </span>
        {participant.is_self && (
          <span className={compact ? "sr-only" : "text-text/50"}>
            ({t("meetings.people.you")})
          </span>
        )}
      </button>
      {open &&
        createPortal(
          <div
            ref={panelRef}
            role="dialog"
            aria-label={participant.name}
            data-testid="person-popover"
            style={{
              position: "fixed",
              left: pos?.left ?? -9999,
              top: pos?.top ?? -9999,
              width: PANEL_WIDTH,
            }}
            className="z-50 space-y-2 rounded-lg border border-mid-gray/30 bg-background p-3 text-sm shadow-lg"
          >
            <div className="space-y-0.5">
              <p
                className="break-words font-semibold"
                data-testid="person-name"
              >
                {participant.name}
              </p>
              <p className="text-xs text-text/60">
                {t(`meetings.people.role.${participant.role}`, {
                  defaultValue: participant.role,
                })}
                {" · "}
                {t(`meetings.people.source.${participant.source}`, {
                  defaultValue: participant.source,
                })}
              </p>
            </div>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-xs">
              <dt className="text-text/60">
                {t("meetings.people.popover.email")}
              </dt>
              <dd className="break-all" data-testid="person-email">
                {participant.email ?? (
                  <span className="text-text/50">
                    {t("meetings.people.popover.noEmail")}
                  </span>
                )}
              </dd>
              {participant.company && (
                <>
                  <dt className="text-text/60">
                    {t("meetings.people.popover.company")}
                  </dt>
                  <dd className="break-all" data-testid="person-company">
                    {participant.company}
                  </dd>
                </>
              )}
            </dl>
            <div className="flex flex-col gap-1.5 border-t border-mid-gray/20 pt-2">
              <Button
                variant="secondary"
                size="sm"
                disabled={count === 0}
                onClick={() => run(() => onFilter(person))}
                title={t("meetings.people.popover.meetingsFilterTitle", {
                  name: participant.name,
                })}
                data-testid="person-meetings"
              >
                <Users width={12} height={12} aria-hidden="true" />
                {t("meetings.people.popover.meetingsWith", {
                  count,
                  name: participant.name,
                })}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={count === 0}
                onClick={() => run(() => onAsk(person))}
                title={t("meetings.people.popover.askTitle", {
                  name: participant.name,
                })}
                data-testid="person-ask"
              >
                <MessageSquare width={12} height={12} aria-hidden="true" />
                {t("meetings.people.popover.ask")}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => run(onManage)}
                data-testid="person-manage"
              >
                {t("meetings.people.popover.manage")}
              </Button>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
};
