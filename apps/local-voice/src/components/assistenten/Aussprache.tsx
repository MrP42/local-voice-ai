import React, { useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react";
import type {
  AusspracheEintrag,
  FehlerCode,
  Profil,
  ProfilFehler,
} from "@/components/voice-call/assistenten";
import {
  GRENZEN,
  ausspracheAendern,
  ausspracheBefunde,
  ausspracheEntfernen,
  ausspracheHinzu,
  ausspracheListe,
  ausspracheVerschieben,
  type ZeilenBefund,
} from "@/components/voice-call/profil/editor";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { SettingsGroup } from "../ui/SettingsGroup";

const KEIN_BEFUND: ZeilenBefund = { wort: [], aussprache: [], zeile: [] };

/** Ein Befund der Kernprüfung als Satz für ein Feld der Zeile; was keinen eigenen Satz hat, sagt der allgemeine Text des Codes. */
function satz(t: TFunction, code: FehlerCode): string {
  return t(`assistenten.aussprache.fehler.${code}`, {
    defaultValue: t(`assistenten.code.${code}`),
    max: GRENZEN.ausspracheText,
  });
}

/**
 * Ein leeres Feld meldet „Länge“ erst, wenn man es verlassen hat: eine eben angelegte Zeile ist noch nicht falsch. Alles andere
 * (zu lang, nicht erlaubte Zeichen, doppelt) steht sofort da. Gespeichert wird nie, solange etwas offen ist (`pruefe`).
 */
function sichtbar(
  codes: FehlerCode[],
  wert: string,
  beruehrt: boolean,
): FehlerCode[] {
  return codes.filter((c) => c !== "laenge" || wert !== "" || beruehrt);
}

interface ZeileProps {
  nummer: number;
  eintrag: AusspracheEintrag;
  befund: ZeilenBefund;
  letzte: boolean;
  beruehrt: { wort: boolean; aussprache: boolean };
  onAendern: (feld: "wort" | "aussprache", wert: string) => void;
  onVerlassen: (feld: "wort" | "aussprache") => void;
  onVerschieben: (richtung: -1 | 1) => void;
  onEntfernen: () => void;
}

const Zeile: React.FC<ZeileProps> = ({
  nummer,
  eintrag,
  befund,
  letzte,
  beruehrt,
  onAendern,
  onVerlassen,
  onVerschieben,
  onEntfernen,
}) => {
  const { t } = useTranslation();
  const id = useId();
  const n = nummer + 1;
  const wortSaetze = sichtbar(befund.wort, eintrag.wort, beruehrt.wort).map(
    (c) => satz(t, c),
  );
  const ausSaetze = sichtbar(
    befund.aussprache,
    eintrag.aussprache,
    beruehrt.aussprache,
  ).map((c) => satz(t, c));
  const zeilenSaetze = befund.zeile.map((c) => satz(t, c));
  return (
    <li
      className="lv-asst-regel"
      data-testid={`assistent-aussprache-zeile-${nummer}`}
    >
      <div className="lv-asst-regel__feld">
        <label htmlFor={`${id}-w`} className="lv-asst-regel__label">
          {t("assistenten.aussprache.wort.label")}
          <span className="lv-asst-sr">
            {" "}
            {t("assistenten.aussprache.zeile", { n })}
          </span>
        </label>
        <Input
          id={`${id}-w`}
          value={eintrag.wort}
          autoComplete="off"
          spellCheck={false}
          placeholder={t("assistenten.aussprache.wort.beispiel")}
          aria-invalid={wortSaetze.length > 0 ? true : undefined}
          aria-describedby={wortSaetze.length > 0 ? `${id}-wf` : undefined}
          onChange={(e) => onAendern("wort", e.target.value)}
          onBlur={() => onVerlassen("wort")}
          data-testid={`assistent-aussprache-wort-${nummer}`}
        />
        {wortSaetze.length > 0 && (
          <div id={`${id}-wf`}>
            {wortSaetze.map((s) => (
              <p
                key={s}
                className="lv-asst-fehlerzeile"
                data-testid={`assistent-aussprache-wort-fehler-${nummer}`}
              >
                {s}
              </p>
            ))}
          </div>
        )}
      </div>
      <div className="lv-asst-regel__feld">
        <label htmlFor={`${id}-a`} className="lv-asst-regel__label">
          {t("assistenten.aussprache.aussprache.label")}
          <span className="lv-asst-sr">
            {" "}
            {t("assistenten.aussprache.zeile", { n })}
          </span>
        </label>
        <Input
          id={`${id}-a`}
          value={eintrag.aussprache}
          autoComplete="off"
          spellCheck={false}
          placeholder={t("assistenten.aussprache.aussprache.beispiel")}
          aria-invalid={ausSaetze.length > 0 ? true : undefined}
          aria-describedby={ausSaetze.length > 0 ? `${id}-af` : undefined}
          onChange={(e) => onAendern("aussprache", e.target.value)}
          onBlur={() => onVerlassen("aussprache")}
          data-testid={`assistent-aussprache-aussprache-${nummer}`}
        />
        {ausSaetze.length > 0 && (
          <div id={`${id}-af`}>
            {ausSaetze.map((s) => (
              <p
                key={s}
                className="lv-asst-fehlerzeile"
                data-testid={`assistent-aussprache-aussprache-fehler-${nummer}`}
              >
                {s}
              </p>
            ))}
          </div>
        )}
      </div>
      <div
        className="lv-asst-regel__aktionen"
        role="group"
        aria-label={t("assistenten.aussprache.zeile", { n })}
      >
        <Button
          variant="ghost"
          size="sm"
          type="button"
          icon={<ArrowUp width={16} height={16} aria-hidden="true" />}
          disabledFocusable={nummer === 0}
          aria-label={t("assistenten.aussprache.hoch", { n })}
          title={t("assistenten.aussprache.hoch", { n })}
          onClick={() => onVerschieben(-1)}
          data-testid={`assistent-aussprache-hoch-${nummer}`}
        />
        <Button
          variant="ghost"
          size="sm"
          type="button"
          icon={<ArrowDown width={16} height={16} aria-hidden="true" />}
          disabledFocusable={letzte}
          aria-label={t("assistenten.aussprache.runter", { n })}
          title={t("assistenten.aussprache.runter", { n })}
          onClick={() => onVerschieben(1)}
          data-testid={`assistent-aussprache-runter-${nummer}`}
        />
        <Button
          variant="ghost"
          size="sm"
          type="button"
          icon={<Trash2 width={16} height={16} aria-hidden="true" />}
          aria-label={t("assistenten.aussprache.entfernen", { n })}
          title={t("assistenten.aussprache.entfernen", { n })}
          onClick={onEntfernen}
          data-testid={`assistent-aussprache-entfernen-${nummer}`}
        />
      </div>
      {zeilenSaetze.length > 0 && (
        <div className="lv-asst-regel__zeile">
          {zeilenSaetze.map((s) => (
            <p key={s} className="lv-asst-fehlerzeile">
              {s}
            </p>
          ))}
        </div>
      )}
    </li>
  );
};

interface Props {
  profil: Profil;
  /** Alle Fehler der Prüfung des Entwurfs (hier zählen die zu `aussprache`). */
  fehler: readonly ProfilFehler[];
  onAendern: (aenderung: (p: Profil) => Profil) => void;
}

/**
 * Die Ausspracheregeln des Assistenten: Tabelle Wort / Aussprache mit Hinzufügen, Entfernen und Reihenfolge. Höchstens 50
 * Regeln; jede Zeile zeigt die Fehler, die die Prüfung des Kerns meldet, nichts wird gekürzt oder still repariert. Eine leere
 * Liste lässt das Feld des Profils wegfallen (nie `[]`), unbekannte Felder einer Regel bleiben beim Ändern und Verschieben.
 */
export const Aussprache: React.FC<Props> = ({ profil, fehler, onAendern }) => {
  const { t } = useTranslation();
  const liste = ausspracheListe(profil);
  const befunde = useMemo(() => ausspracheBefunde(fehler), [fehler]);
  const voll = liste.length >= GRENZEN.aussprache;
  const [beruehrt, setBeruehrt] = useState<Set<string>>(() => new Set());
  const [angesagt, setAngesagt] = useState("");
  // Nach Hinzufügen, Entfernen und Verschieben wandert der Fokus mit: sonst bliebe er auf einem Knopf, der etwas anderes meint.
  const fokus = useRef<string | null>(null);
  const hinzuId = useId();
  const vollId = useId();
  const leereZeilen = liste.filter(
    (e) => e.wort === "" || e.aussprache === "",
  ).length;

  useEffect(() => {
    const ziel = fokus.current;
    if (ziel === null) return;
    fokus.current = null;
    document.querySelector<HTMLElement>(`[data-testid="${ziel}"]`)?.focus();
  }, [liste]);

  const strukturell = () => setBeruehrt(new Set());
  const merke = (nummer: number, feld: "wort" | "aussprache") =>
    setBeruehrt((alt) => new Set(alt).add(`${nummer}:${feld}`));

  const hinzu = () => {
    if (voll) return;
    strukturell();
    fokus.current = `assistent-aussprache-wort-${liste.length}`;
    setAngesagt(
      t("assistenten.aussprache.angesagt.hinzu", { n: liste.length + 1 }),
    );
    onAendern(ausspracheHinzu);
  };
  const entfernen = (i: number) => {
    strukturell();
    // Danach steht die nächste Zeile an dieser Stelle; war es die letzte, geht der Fokus auf die davor oder auf „Hinzufügen“.
    fokus.current =
      liste.length === 1
        ? "assistent-aussprache-hinzu"
        : `assistent-aussprache-wort-${Math.min(i, liste.length - 2)}`;
    setAngesagt(t("assistenten.aussprache.angesagt.entfernt", { n: i + 1 }));
    onAendern((p) => ausspracheEntfernen(p, i));
  };
  const verschieben = (i: number, richtung: -1 | 1) => {
    const neu = i + richtung;
    if (neu < 0 || neu >= liste.length) return;
    strukturell();
    // Der Fokus bleibt auf dem Knopf derselben Richtung, jetzt an der neuen Stelle (am Rand sperrt er, bleibt aber erreichbar).
    fokus.current = `assistent-aussprache-${richtung === -1 ? "hoch" : "runter"}-${neu}`;
    setAngesagt(
      t("assistenten.aussprache.angesagt.verschoben", {
        n: i + 1,
        stelle: neu + 1,
      }),
    );
    onAendern((p) => ausspracheVerschieben(p, i, richtung));
  };

  return (
    <SettingsGroup
      title={t("assistenten.abschnitt.aussprache")}
      description={t("assistenten.aussprache.beschreibung")}
    >
      <div className="lv-asst-block" data-testid="assistent-aussprache">
        <div className="lv-asst-feld__zeile">
          <p className="lv-asst-hinweis m-0 min-w-0 flex-1">
            {t("assistenten.aussprache.hinweis")}
          </p>
          <span
            className="lv-asst-zaehler"
            data-stand={
              befunde.liste.length > 0 || liste.length > GRENZEN.aussprache
                ? "fehler"
                : voll
                  ? "warn"
                  : "ok"
            }
            data-testid="assistent-aussprache-zaehler"
          >
            {t("assistenten.aussprache.zaehler", {
              n: liste.length,
              max: GRENZEN.aussprache,
            })}
          </span>
        </div>

        {befunde.liste.map((c) => (
          <p
            key={c}
            className="lv-asst-fehlerzeile"
            data-testid="assistent-aussprache-listenfehler"
          >
            {t(`assistenten.aussprache.fehler.liste.${c}`, {
              defaultValue: t(`assistenten.code.${c}`),
              max: GRENZEN.aussprache,
            })}
          </p>
        ))}

        {liste.length === 0 ? (
          <p
            className="lv-asst-hinweis"
            data-testid="assistent-aussprache-leer"
          >
            {t("assistenten.aussprache.leer")}
          </p>
        ) : (
          <div className="lv-asst-tabelle">
            <div className="lv-asst-regeln__kopf" aria-hidden="true">
              <span>{t("assistenten.aussprache.wort.label")}</span>
              <span>{t("assistenten.aussprache.aussprache.label")}</span>
              <span />
            </div>
            <ol
              className="lv-asst-regeln"
              aria-label={t("assistenten.aussprache.liste")}
              data-testid="assistent-aussprache-liste"
            >
              {liste.map((e, i) => (
                <Zeile
                  // Die Regeln haben keine eigene Kennung; die Eingaben sind gesteuert, die Stelle genügt als Schlüssel.
                  key={i}
                  nummer={i}
                  eintrag={e}
                  befund={befunde.zeilen[i] ?? KEIN_BEFUND}
                  letzte={i === liste.length - 1}
                  beruehrt={{
                    wort: beruehrt.has(`${i}:wort`),
                    aussprache: beruehrt.has(`${i}:aussprache`),
                  }}
                  onAendern={(feld, wert) =>
                    onAendern((p) => ausspracheAendern(p, i, feld, wert))
                  }
                  onVerlassen={(feld) => merke(i, feld)}
                  onVerschieben={(r) => verschieben(i, r)}
                  onEntfernen={() => entfernen(i)}
                />
              ))}
            </ol>
          </div>
        )}

        {leereZeilen > 0 && (
          <p
            className="lv-asst-hinweis"
            data-testid="assistent-aussprache-offen"
          >
            {t("assistenten.aussprache.unvollstaendig")}
          </p>
        )}

        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="secondary"
            size="sm"
            type="button"
            icon={<Plus width={16} height={16} aria-hidden="true" />}
            disabledFocusable={voll}
            aria-describedby={voll ? vollId : hinzuId}
            onClick={hinzu}
            data-testid="assistent-aussprache-hinzu"
          >
            {t("assistenten.aussprache.hinzu")}
          </Button>
          <span
            id={voll ? vollId : hinzuId}
            className="lv-asst-hinweis"
            data-testid={voll ? "assistent-aussprache-voll" : undefined}
          >
            {voll
              ? t("assistenten.aussprache.voll", { max: GRENZEN.aussprache })
              : t("assistenten.aussprache.hinzuHinweis", {
                  max: GRENZEN.ausspracheText,
                })}
          </span>
        </div>
        <p
          className="lv-asst-sr"
          role="status"
          data-testid="assistent-aussprache-angesagt"
        >
          {angesagt}
        </p>
      </div>
    </SettingsGroup>
  );
};
