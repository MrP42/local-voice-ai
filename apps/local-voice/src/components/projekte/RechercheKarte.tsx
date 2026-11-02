import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Sparkles } from "lucide-react";
import { commands, type RechercheSicht } from "@/bindings";
import { auspacken, fehlerText, hostVon } from "@/lib/projekte";
import { ReadAloudButton } from "../settings/meetings/ReadAloudButton";

interface Props {
  /** Kennung des Recherche-Dokuments. */
  id: string;
  /** Hinweis, dass das Ergebnis im Projektordner liegt (Name des Projekts). */
  projektName?: string;
  onClose?: () => void;
}

/**
 * Kompaktes Ergebnis einer Recherche (Modus Schnell) für die Chat-Spalte und das Sprachgespräch:
 * Frage, Antwort mit Quellenmarken, Quellenliste, Kennzeichnung "KI-erstellt" und Vorlesen über die
 * vorhandene Sprachausgabe (`ReadAloudButton`, dieselbe Stimme wie auf der Vorlesen-Seite).
 */
export const RechercheKarte: React.FC<Props> = ({
  id,
  projektName,
  onClose,
}) => {
  const { t } = useTranslation();
  const [doc, setDoc] = useState<RechercheSicht | null>(null);
  const [fehler, setFehler] = useState<string | null>(null);

  useEffect(() => {
    let weg = false;
    setDoc(null);
    setFehler(null);
    commands
      .rechercheLesen(id)
      .then((r) => {
        if (!weg) setDoc(auspacken(r));
      })
      .catch((e) => {
        if (!weg) setFehler(fehlerText(e));
      });
    return () => {
      weg = true;
    };
  }, [id]);

  if (fehler) {
    return (
      <p role="alert" className="text-xs text-red-500">
        {fehler}
      </p>
    );
  }
  if (!doc) return null;

  return (
    <section
      data-testid="recherche-karte"
      aria-label={t("projekte.karte.titel")}
      className="space-y-2 rounded-lg border border-mid-gray/30 bg-mid-gray/5 p-2.5 text-sm"
    >
      <header className="flex items-start gap-2">
        <Sparkles
          width={14}
          height={14}
          className="mt-0.5 shrink-0 text-logo-primary"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1">
          <p className="text-xs text-text/60">
            {t("projekte.karte.kennzeichen", { klasse: doc.dataClassLabel })}
          </p>
          <p className="break-words font-medium" data-testid="recherche-frage">
            {doc.question}
          </p>
        </div>
        {onClose && (
          <button
            type="button"
            onClick={onClose}
            className="shrink-0 rounded px-1.5 text-xs text-text/60 hover:bg-mid-gray/15 cursor-pointer"
          >
            {t("projekte.karte.schliessen")}
          </button>
        )}
      </header>
      {doc.answer.length === 0 ? (
        <p className="text-xs text-text/60">
          {t("projekte.karte.keineAntwort")}
        </p>
      ) : (
        <p className="break-words leading-relaxed">
          {doc.answer.map((a, i) => (
            <span
              key={i}
              className={a.unsupported ? "text-text/60" : undefined}
            >
              {a.text}
              {a.citations.map((c) => (
                <sup key={c} className="ms-0.5 text-[10px] text-logo-primary">
                  [{c}]
                </sup>
              ))}
              {a.unsupported && (
                <span className="ms-1 text-[10px] uppercase text-text/50">
                  {t("projekte.karte.ohneQuelle")}
                </span>
              )}{" "}
            </span>
          ))}
        </p>
      )}
      {doc.sources.length > 0 && (
        <ol className="space-y-0.5 text-xs text-text/70">
          {doc.sources.map((q) => (
            <li key={q.index} className="min-w-0 truncate">
              <span className="text-logo-primary">[{q.index}]</span> {q.title}{" "}
              <span className="text-text/50">· {hostVon(q.url)}</span>
            </li>
          ))}
        </ol>
      )}
      <footer className="flex flex-wrap items-center gap-2 text-xs text-text/60">
        <ReadAloudButton
          speakKey={`recherche:${doc.id}`}
          getMarkdown={() => doc.readAloud}
          testId="recherche-readaloud"
          fileStem={doc.question.slice(0, 40)}
        />
        {projektName && (
          <span data-testid="recherche-abgelegt">
            {t("projekte.karte.abgelegt", { projekt: projektName })}
          </span>
        )}
      </footer>
    </section>
  );
};
