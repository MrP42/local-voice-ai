export {
  BEKANNTE_BEREICHE,
  FEHLER_CODES,
  PROFIL_SCHEMA,
  istGeraetename,
  kodiereJson,
  kodiereProfil,
  kontextModusWirksam,
  neuesProfil,
  parseJson,
  parseProfil,
  profilAusBaum,
  profilBaum,
  pruefeProfil,
  wirksameBereiche,
} from "./profil";
export type {
  Auswertung,
  AuswertungFeld,
  FehlerCode,
  JsonMitglied,
  JsonWert,
  Kontext,
  PcBereich,
  Profil,
  ProfilErgebnis,
  ProfilFehler,
  Verhalten,
  Wissen,
} from "./profil";
export {
  istEnglisch,
  loeseBegruessung,
  loesePlatzhalter,
  rollenblock,
} from "./text";
export type {
  PlatzhalterErgebnis,
  PlatzhalterWerte,
  Rollenblock,
} from "./text";
export {
  ALLES_ID,
  VORLAGEN,
  alsEigenenKopieren,
  istVorbehalteneId,
  istVorlage,
  vorlage,
} from "./vorlagen";
