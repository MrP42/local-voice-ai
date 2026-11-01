import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { ProfilseiteLabels } from "./Profilseite";
import type { KontoFeld } from "./profil";

const FELDER: readonly KontoFeld[] = [
  "anzeigename",
  "vorname",
  "nachname",
  "email",
  "telefon",
  "mobil",
  "position",
  "abteilung",
  "organisation",
  "standort",
];

/** Beschriftungen der Konto-Bausteine aus den Übersetzungen (`konto.*`). */
export function useKontoLabels(): ProfilseiteLabels {
  const { t } = useTranslation();
  return useMemo(
    () => ({
      lokalesProfil: t("konto.lokalesProfil"),
      status: {
        lokal: t("konto.status.lokal"),
        angemeldet: t("konto.status.angemeldet"),
        abgelaufen: t("konto.status.abgelaufen"),
      },
      suite: t("konto.suite"),
      anmelden: t("konto.anmelden"),
      erneutAnmelden: t("konto.erneutAnmelden"),
      abmelden: t("konto.abmelden"),
      abmeldenAlle: t("konto.abmeldenAlle"),
      angemeldetBei: (anbieter: string) =>
        t("konto.angemeldetBei", { anbieter }),
      meinProfil: t("konto.meinProfil"),
      kontoGeraete: t("konto.kontoGeraete"),
      profilHinzufuegen: t("konto.profilHinzufuegen"),
      profilWechseln: (name: string) => t("konto.profilWechseln", { name }),
      gesperrt: t("konto.gesperrt"),
      anmeldenHinweisTitel: t("konto.anmeldenHinweisTitel"),
      anmeldenHinweis: t("konto.anmeldenHinweis"),
      abgelaufenHinweis: t("konto.abgelaufenHinweis"),
      profilmenue: (name: string) => t("konto.profilmenue", { name }),
      bildVon: (name: string) => t("konto.bildVon", { name }),
      beschreibung: t("konto.beschreibung"),
      bildAendern: t("konto.bildAendern"),
      bildEntfernen: t("konto.bildEntfernen"),
      bildZuGross: t("konto.bildZuGross"),
      bildTyp: t("konto.bildTyp"),
      bildFehler: t("konto.bildFehler"),
      kontaktdaten: t("konto.kontaktdaten"),
      speichern: t("konto.speichern"),
      gespeichert: t("konto.gespeichert"),
      vonOrganisation: t("konto.vonOrganisation"),
      herkunft: (anbieter: string) => t("konto.herkunft", { anbieter }),
      felder: Object.fromEntries(
        FELDER.map((f) => [f, t(`konto.felder.${f}`)]),
      ) as Record<KontoFeld, string>,
      anmeldung: t("konto.anmeldung"),
      anmeldungLokal: t("konto.anmeldungLokal"),
      anmeldungLokalNutzen: t("konto.anmeldungLokalNutzen"),
      anmeldungAktiv: t("konto.anmeldungAktiv"),
      anmeldungAbgelaufen: t("konto.anmeldungAbgelaufen"),
      anbieter: t("konto.anbieter"),
      seit: t("konto.seit"),
      apps: t("konto.apps"),
      emailUngueltig: t("konto.emailUngueltig"),
    }),
    [t],
  );
}
