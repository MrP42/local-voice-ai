import { useEffect, useState } from "react";
import type { BookTalkCitation } from "@/bindings";
import { CHANGED_EVENT, HYDRATED_EVENT } from "@/lib/library/progress";
import { useSettings } from "../../../../hooks/useSettings";
import { tauriBookTalkPort } from "./api";
import { BookTalkColumn } from "./BookTalkColumn";
import { ConsentDialog } from "./ConsentDialog";
import { pageAt } from "./pages";
import type { BookTalkPort } from "./port";
import type { Turn } from "./turns";
import { useBookTalk } from "./useBookTalk";
import { useBookVoice, type BookTalkHost } from "./useBookVoice";

/**
 * Gesprächsspalte mit allem, was sie braucht (lva#309): Verlauf und Anfragen,
 * Sprachgespräch, Cloud-Freigabe. Sitzt in der rechten Reader-Spalte und im
 * Blatt „Gespräch“ des schmalen Fensters; beim Schließen endet die Sitzung,
 * ein erweiterter Bereich („Ganzes Buch“) fällt auf „bis hier“ zurück.
 */
export function BookTalkPanel({
  bookId,
  text,
  frontier,
  host,
  port = tauriBookTalkPort,
  keyboard = true,
}: {
  bookId: string;
  /** Originaltext des Buchs (für die Seitenzahl an der Lesegrenze). */
  text: string;
  /** Lesegrenze (UTF-16); `undefined`, solange sie unbekannt ist. */
  frontier(): number | undefined;
  host: BookTalkHost;
  port?: BookTalkPort;
  keyboard?: boolean;
}) {
  const { settings } = useSettings();
  const [boundary, setBoundary] = useState<number | undefined>(() => frontier());
  const [consent, setConsent] = useState<Turn | null>(null);

  // Die Grenze wächst mit dem Lesen und Hören; der Fortschrittsspeicher meldet jede Änderung.
  useEffect(() => {
    const read = () => setBoundary(frontier());
    read();
    window.addEventListener(CHANGED_EVENT, read);
    window.addEventListener(HYDRATED_EVENT, read);
    return () => {
      window.removeEventListener(CHANGED_EVENT, read);
      window.removeEventListener(HYDRATED_EVENT, read);
    };
  }, [frontier, bookId]);

  const talk = useBookTalk({ port, bookId, frontier });
  const voice = useBookVoice({ port, bookId, talk, frontier, host, keyboard });

  const jump = (c: BookTalkCitation) => host.jumpTo(c);
  const readFrom = (c: BookTalkCitation) => {
    // Die Wiedergabe beginnt am Beleg; ein laufendes Gespräch endet dafür.
    if (voice.phase !== "off" || voice.readingPaused) voice.end(false);
    host.readFrom(c.start);
  };
  const pausedAt = voice.readingPaused ? host.position() : null;

  return (
    <>
      <BookTalkColumn
        talk={talk}
        voice={voice}
        page={boundary !== undefined ? pageAt(text, boundary) : null}
        pausedPage={pausedAt !== null ? pageAt(text, pausedAt) : null}
        enabled={settings?.book_talk_enabled ?? false}
        onJump={jump}
        onReadFrom={readFrom}
        onConsent={setConsent}
      />
      <ConsentDialog
        open={consent !== null}
        onClose={() => setConsent(null)}
        onGranted={() => {
          const turn = consent;
          setConsent(null);
          if (turn) talk.askAgain(turn);
        }}
      />
    </>
  );
}
