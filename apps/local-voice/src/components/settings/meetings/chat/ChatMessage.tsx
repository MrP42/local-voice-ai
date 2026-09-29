import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { ChatMessage as ChatMessageData, Citation } from "@/bindings";
import { splitAnswer } from "@/lib/meetingChat";
import { CitationChip } from "./CitationChip";
import { CoverageNote } from "./CoverageNote";

interface ChatMessageProps {
  message: ChatMessageData;
  /** Chat ueber viele Besprechungen (Coverage-Formulierung). */
  global: boolean;
  onJump?: (citation: Citation) => void;
}

/**
 * Eine Nachricht im Verlauf. Antworten zeigen Belege als Chips `[n]`, darunter
 * grau die Abdeckung; "nichts gefunden" und "ohne Beleg" sind eigene Hinweise,
 * damit eine unbelegte Antwort nie wie eine belegte aussieht.
 */
export const ChatMessage: React.FC<ChatMessageProps> = ({
  message,
  global,
  onJump,
}) => {
  const { t } = useTranslation();
  const citations = useMemo(() => message.citations ?? [], [message.citations]);
  const parts = useMemo(
    () => splitAnswer(message.text ?? "", new Set(citations.map((c) => c.n))),
    [message.text, citations],
  );

  if (message.role === "user") {
    return (
      <div className="flex justify-end" data-testid="chat-question">
        <p className="max-w-[85%] whitespace-pre-wrap break-words rounded-lg bg-logo-primary/15 px-3 py-1.5 text-sm text-text">
          {message.text}
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-1" data-testid="chat-answer">
      {message.not_found ? (
        <p data-testid="chat-not-found" className="text-sm text-text/70 italic">
          {t("meetings.chat.notFound")}
        </p>
      ) : (
        <p
          data-citation-scope
          className="whitespace-pre-wrap break-words text-sm leading-relaxed text-text"
        >
          {parts.map((part, i) =>
            part.kind === "text" ? (
              <React.Fragment key={i}>{part.text}</React.Fragment>
            ) : (
              <CitationChip
                key={i}
                n={part.n}
                citation={citations.find((c) => c.n === part.n)}
                onJump={onJump}
              />
            ),
          )}
          {message.uncited && !message.not_found && (
            <span
              data-testid="chat-uncited"
              title={t("meetings.chat.uncitedHint")}
              className="ml-1.5 inline-block rounded-md border border-amber-500/40 bg-amber-500/10 px-1.5 text-[11px] leading-4 text-amber-700 dark:text-amber-300"
            >
              {t("meetings.chat.uncited")}
            </span>
          )}
        </p>
      )}
      <CoverageNote coverage={message.coverage} global={global} />
    </div>
  );
};
