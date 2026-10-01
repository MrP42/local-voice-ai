/**
 * Eine Person als Verweis (Liste eingrenzen, Fragen zu ihr). Das Popover je
 * Person gibt es seit G4 (#70) nicht mehr: die Teilnehmenden stehen im Chip des
 * Kopfes (`ParticipantsPopover`), mit Rolle, Adresse, Filter und Fragen je Zeile.
 */
export interface PersonRef {
  id: string;
  name: string;
}
