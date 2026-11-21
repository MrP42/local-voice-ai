import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeft } from "lucide-react";
import { useAssistentStore } from "@/stores/assistentStore";
import {
  loeseBegruessung,
  type Profil,
} from "@/components/voice-call/assistenten";
import type { AbgewiesenerAssistent } from "@/components/voice-call/profil/speicher";
import { werteJetzt } from "@/components/voice-call/profil/wirkung";
import {
  GRENZEN,
  bereichsAnzeige,
  bereichsSumme,
  zeichen,
} from "@/components/voice-call/profil/editor";
import { useKnowledge } from "@/components/voice-call/useKnowledge";
import { Button } from "../ui/Button";
import { SettingsGroup } from "../ui/SettingsGroup";
import { AssistentMenue } from "./AssistentMenue";
import { useSprachName } from "./anzeige";
import { fehlerZeilen } from "./fehler";

// ---------------------------------------------------------------- Vorlage (nur ansehen)

interface VorlageProps {
  profil: Profil;
  gewaehlt: boolean;
  /** Schreibende Vorgänge erlaubt (Speicher lesbar, kein Vorgang läuft). */
  schreibt: boolean;
  /** „Im Gespräch verwenden“ ist gesperrt (ein Gespräch läuft oder der Speicher ist nicht lesbar), mit dem Grund als Satz. */
  verwendenGrund: string | null;
  onZurueck: () => void;
  onKopieren: () => void;
  onVerwenden: () => void;
  onExportieren: () => void;
}

/**
 * Eine Vorlage ansehen: alles, was sie festlegt, ohne Eingabefelder. Eine Vorlage wird nie verändert; „Als eigenen kopieren“
 * legt einen eigenen Assistenten daraus an, den man bearbeiten kann.
 */
export const VorlagenAnsicht: React.FC<VorlageProps> = ({
  profil: p,
  gewaehlt,
  schreibt,
  verwendenGrund,
  onZurueck,
  onKopieren,
  onVerwenden,
  onExportieren,
}) => {
  const { t } = useTranslation();
  const sprachName = useSprachName();
  const kontoName = useAssistentStore((s) => s.name);
  const { areas: freigegeben } = useKnowledge();
  const werte = useMemo(() => werteJetzt(kontoName), [kontoName]);
  const bereiche = useMemo(
    () => bereichsAnzeige(p, freigegeben),
    [p, freigegeben],
  );
  const summe = useMemo(() => bereichsSumme(p, freigegeben), [p, freigegeben]);
  const begruessung = loeseBegruessung(p, werte).text;
  const ja = (b: boolean) => (b ? t("assistenten.ja") : t("assistenten.nein"));

  return (
    <article
      className="space-y-4"
      aria-labelledby="asst-ansicht-titel"
      data-testid="assistent-vorlage"
    >
      <header className="lv-asst-kopf">
        <Button
          variant="ghost"
          size="sm"
          className="lv-asst__zurueck"
          onClick={onZurueck}
          type="button"
          data-testid="assistent-zurueck"
        >
          <ArrowLeft width={16} height={16} aria-hidden="true" />
          {t("assistenten.zurueck")}
        </Button>
        <div className="lv-asst-kopf__titel">
          <h2 id="asst-ansicht-titel">{p.name}</h2>
          <p>
            {gewaehlt
              ? t("assistenten.kopf.vorlageGewaehlt")
              : t("assistenten.kopf.vorlage")}
          </p>
          {verwendenGrund && (
            <p
              id="asst-vorlage-grund"
              className="lv-asst-hinweis"
              data-testid="assistent-verwenden-grund"
            >
              {verwendenGrund}
            </p>
          )}
        </div>
        <div className="lv-asst-kopf__aktionen">
          <Button
            variant="primary"
            size="sm"
            type="button"
            onClick={onKopieren}
            disabledFocusable={!schreibt}
            data-testid="assistent-vorlage-kopieren"
          >
            {t("assistenten.aktion.alsEigenen")}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            type="button"
            onClick={onVerwenden}
            disabledFocusable={verwendenGrund !== null}
            aria-describedby={verwendenGrund ? "asst-vorlage-grund" : undefined}
            data-testid="assistent-verwenden"
          >
            {t("assistenten.aktion.verwenden")}
          </Button>
          <AssistentMenue
            name={p.name}
            art="vorlage"
            schreibt={schreibt}
            onKopieren={onKopieren}
            onExportieren={onExportieren}
            testId="assistent-vorlage-menue"
            className="lv-btn lv-asst-zeile__menue"
          />
        </div>
      </header>

      <div className="lv-asst-abschnitte">
        <SettingsGroup title={t("assistenten.abschnitt.allgemein")}>
          <div className="lv-asst-block">
            <dl className="lv-asst-dl">
              <dt>{t("assistenten.name.label")}</dt>
              <dd>{p.name}</dd>
              <dt>{t("assistenten.beschreibung.label")}</dt>
              <dd>{p.beschreibung || t("assistenten.leer")}</dd>
            </dl>
          </div>
        </SettingsGroup>

        <SettingsGroup title={t("assistenten.abschnitt.anweisungen")}>
          <div className="lv-asst-block">
            <p className="lv-asst-hinweis">
              {t("assistenten.zaehler", {
                n: zeichen(p.anweisungen),
                max: GRENZEN.anweisungen,
              })}
            </p>
            <pre
              className="lv-asst-text"
              data-testid="assistent-vorlage-anweisungen"
            >
              {p.anweisungen || t("assistenten.leer")}
            </pre>
          </div>
        </SettingsGroup>

        <SettingsGroup title={t("assistenten.abschnitt.begruessung")}>
          <div className="lv-asst-block">
            <p className="lv-asst-text">
              {begruessung || t("assistenten.begruessung.keine")}
            </p>
          </div>
        </SettingsGroup>

        <SettingsGroup title={t("assistenten.abschnitt.stimme")}>
          <div className="lv-asst-block">
            <dl className="lv-asst-dl">
              <dt>{t("assistenten.sprache.label")}</dt>
              <dd>{sprachName(p.sprache)}</dd>
              <dt>{t("assistenten.stimme.label")}</dt>
              <dd>{p.stimme ?? t("assistenten.stimme.standard")}</dd>
            </dl>
          </div>
        </SettingsGroup>

        <SettingsGroup title={t("assistenten.abschnitt.verhalten")}>
          <div className="lv-asst-block">
            <dl className="lv-asst-dl">
              <dt>{t("assistenten.verhalten.stil.label")}</dt>
              <dd>{t(`assistenten.verhalten.stil.${p.verhalten.stil}`)}</dd>
              <dt>{t("assistenten.verhalten.pause.label")}</dt>
              <dd>
                {t(`assistenten.verhalten.pause.${p.verhalten.sprechpause}`)}
              </dd>
              <dt>{t("assistenten.verhalten.unterbrechen.label")}</dt>
              <dd>{ja(p.verhalten.unterbrechen)}</dd>
              <dt>{t("assistenten.verhalten.nachfrage.label")}</dt>
              <dd>
                {p.verhalten.nachfrageStilleS === 0
                  ? t("assistenten.verhalten.nachfrage.aus")
                  : t("assistenten.verhalten.nachfrage.sekunden", {
                      count: p.verhalten.nachfrageStilleS,
                    })}
              </dd>
            </dl>
          </div>
        </SettingsGroup>

        <SettingsGroup
          title={t("assistenten.abschnitt.wissen")}
          description={t("assistenten.wissen.regel")}
        >
          <div className="lv-asst-block">
            {p.wissen.alleBereiche ? (
              <p className="m-0 text-sm">
                {t("assistenten.wissen.alle.label")}
              </p>
            ) : (
              <ul
                className="m-0 ps-5 text-sm"
                data-testid="assistent-vorlage-bereiche"
              >
                {bereiche
                  .filter((z) => z.gewollt)
                  .map((z) => (
                    <li key={z.schluessel}>
                      {t(`assistenten.wissen.bereich.${z.schluessel}`, {
                        defaultValue: z.schluessel,
                      })}
                    </li>
                  ))}
              </ul>
            )}
            <p className="lv-asst-hinweis">
              {t("assistenten.wissen.summe", {
                wirksam: summe.wirksam,
                frei: summe.freigegeben,
              })}
            </p>
          </div>
        </SettingsGroup>

        {p.aussprache && p.aussprache.length > 0 && (
          <SettingsGroup title={t("assistenten.abschnitt.aussprache")}>
            <div className="lv-asst-block">
              <dl
                className="lv-asst-dl"
                data-testid="assistent-vorlage-aussprache"
              >
                {p.aussprache.map((e) => (
                  <React.Fragment key={e.wort}>
                    <dt>{e.wort}</dt>
                    <dd>{e.aussprache}</dd>
                  </React.Fragment>
                ))}
              </dl>
            </div>
          </SettingsGroup>
        )}
      </div>
    </article>
  );
};

// ---------------------------------------------------------------- Abgewiesene Datei

interface AbgewiesenProps {
  datei: AbgewiesenerAssistent;
  onZurueck: () => void;
}

/**
 * Eine Datei im Ordner der Assistenten, die der Speicher abgewiesen hat: die Fehlerliste des Kerns, Feld für Feld. Die Datei
 * bleibt, wie sie ist: nichts wird repariert, überschrieben oder gelöscht.
 */
export const AbgewiesenAnsicht: React.FC<AbgewiesenProps> = ({
  datei,
  onZurueck,
}) => {
  const { t } = useTranslation();
  const zeilen = fehlerZeilen(t, datei.fehler);
  return (
    <article
      className="space-y-4"
      aria-labelledby="asst-abgewiesen-titel"
      data-testid="assistent-abgewiesen-ansicht"
    >
      <header className="lv-asst-kopf">
        <Button
          variant="ghost"
          size="sm"
          className="lv-asst__zurueck"
          onClick={onZurueck}
          type="button"
          data-testid="assistent-zurueck"
        >
          <ArrowLeft width={16} height={16} aria-hidden="true" />
          {t("assistenten.zurueck")}
        </Button>
        <div className="lv-asst-kopf__titel">
          <h2 id="asst-abgewiesen-titel">{datei.datei}</h2>
          <p>{t("assistenten.abgewiesen.kopf")}</p>
        </div>
      </header>
      <div className="lv-asst-meldung" data-art="fehler">
        <h3>{t("assistenten.abgewiesen.titel")}</h3>
        <p className="m-0">{t("assistenten.abgewiesen.text")}</p>
        {zeilen.length > 0 && (
          <ul data-testid="assistent-abgewiesen-fehler">
            {zeilen.map((z) => (
              <li key={z}>{z}</li>
            ))}
          </ul>
        )}
        <p className="lv-asst-hinweis">
          {t("assistenten.abgewiesen.unberuehrt")}
        </p>
      </div>
    </article>
  );
};
