/**
 * Fehler- und Warnmeldungen bündeln (lva#654): gleiche Meldungen erscheinen
 * nicht als Stapel identischer Kästen, sondern als EINE Meldung mit Zähler
 * („… (3×)“). Ohne Abhängigkeit von sonner, damit es ohne DOM testbar ist:
 * {@link buendeln} umhüllt eine Funktion mit der Signatur von `toast.error`.
 *
 * Eine Meldung mit eigener `id` steuert der Aufrufer selbst; sie bleibt
 * unberührt. Nur Texte werden gebündelt, React-Inhalte gehen unverändert durch.
 */

export interface MeldungsAktion {
  label: string;
  onClick: () => void;
}

/** Was `toast.error`/`toast.warning` als zweites Argument nimmt (Ausschnitt). */
export interface MeldungsDaten {
  id?: string | number;
  description?: unknown;
  action?: unknown;
  cancel?: unknown;
  onDismiss?: (t: unknown) => void;
  onAutoClose?: (t: unknown) => void;
  [key: string]: unknown;
}

export type MeldungsFunktion = (
  message: unknown,
  data?: MeldungsDaten,
) => string | number;

/** Gebündelte Meldungsfunktion; `vergessen(id)` nach `toast.dismiss(id)`. */
export type BuendelFunktion = MeldungsFunktion & {
  vergessen: (id?: string | number) => void;
};

export interface BuendelOptionen {
  /** Präfix der vergebenen Kennungen (je Meldungsart eigenes). */
  praefix: string;
  /** Zusatzknopf je Text, z. B. „Grenze ändern“ bei Speicherschutz-Meldungen. */
  aktionFuer?: (text: string) => MeldungsAktion | undefined;
}

interface Eintrag {
  id: string;
  anzahl: number;
}

export function buendeln(
  original: MeldungsFunktion,
  { praefix, aktionFuer }: BuendelOptionen,
): BuendelFunktion {
  const offen = new Map<string, Eintrag>();
  let naechste = 0;

  const gebuendelt: MeldungsFunktion = (message, data) => {
    if (typeof message !== "string") return original(message, data);
    // Eigene Kennung, eigene Aktion oder ein nicht-textlicher Zusatz: der
    // Aufrufer steuert die Meldung selbst, nichts wird zusammengelegt. Der
    // Speicherschutz-Knopf kommt trotzdem dazu, wenn keine Aktion da ist.
    if (
      data?.id !== undefined ||
      data?.action !== undefined ||
      data?.cancel !== undefined ||
      data?.onDismiss !== undefined ||
      data?.onAutoClose !== undefined ||
      (data?.description !== undefined && typeof data.description !== "string")
    ) {
      const knopf =
        data?.action === undefined ? aktionFuer?.(message) : undefined;
      return original(message, knopf ? { ...data, action: knopf } : data);
    }
    const beschreibung =
      typeof data?.description === "string" ? data.description : "";
    const schluessel = `${message}\u0000${beschreibung}`;
    let eintrag = offen.get(schluessel);
    if (eintrag) {
      eintrag.anzahl += 1;
    } else {
      eintrag = { id: `${praefix}-${++naechste}`, anzahl: 1 };
      offen.set(schluessel, eintrag);
    }
    const id = eintrag.id;
    // Nur den eigenen Eintrag entfernen: eine spätere gleiche Meldung kann
    // inzwischen einen neuen Eintrag mit neuer Kennung haben.
    const vergessen = () => {
      if (offen.get(schluessel)?.id === id) offen.delete(schluessel);
    };
    const aktion = aktionFuer?.(message);
    const text =
      eintrag.anzahl > 1 ? `${message} (${eintrag.anzahl}×)` : message;
    return original(text, {
      ...data,
      ...(aktion ? { action: aktion } : {}),
      id,
      onDismiss: (t: unknown) => {
        vergessen();
        data?.onDismiss?.(t);
      },
      onAutoClose: (t: unknown) => {
        vergessen();
        data?.onAutoClose?.(t);
      },
    });
  };

  // Programmatisches Schließen (`toast.dismiss(id)`, ohne id: alle) ruft nicht
  // sicher einen der Rückrufe auf; der Eintrag muss trotzdem weg.
  const vergessenNachId = (id?: string | number) => {
    for (const [schluessel, eintrag] of offen) {
      if (id === undefined || eintrag.id === id) offen.delete(schluessel);
    }
  };
  return Object.assign(gebuendelt, { vergessen: vergessenNachId });
}

/**
 * Meldungen des Speicherschutzes (Sperre, Warnung, Kindprozess-Tor), auch
 * eingebettet in eine Aufrufermeldung („Anbieter nicht verfügbar: …“).
 */
export function istSpeicherschutzMeldung(text: string): boolean {
  return (
    text.includes("Speicherschutz:") ||
    text.includes("Speicher knapp:") ||
    text.includes("Zu wenig freier Arbeitsspeicher")
  );
}
