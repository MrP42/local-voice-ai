import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Menu,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MenuDivider,
} from "@fluentui/react-components";
import { ChevronDown, Clock, UserRound } from "lucide-react";
import { useAssistentStore } from "@/stores/assistentStore";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { useGeltenderAssistent } from "./profil/gilt";
import { aktivesProfil } from "./profil/speicher";
import { ALLES_ID, VORLAGEN } from "./assistenten";
import type { Profil } from "./assistenten";

/**
 * Auswahl des Assistenten im Chatfeld, neben der Modellwahl (lva#556). Sie zeigt „Alles“, die Vorlagen und die eigenen
 * Assistenten als Auswahlmenü (Tastatur und Screenreader wie bei der Modellwahl, Ziele mindestens 44 px auf dem Telefon).
 * Eine Auswahl beginnt ein neues, leeres Gespräch: Rolle, Begrenzung der Bereiche und Begrüßung gehören zum Beginn eines
 * Gesprächs, ein halbes Gespräch bekäme sie nicht. Im Projektgespräch gilt der Assistent für Rolle und Wissen nicht
 * (dort bestimmen die Regeln des Projekts, was die KI sagt und kennt): die Auswahl zeigt das und lässt sich dort nicht
 * öffnen. Trifft eine Auswahl erst nach dem Beginn eines Gesprächs ein (spät geladen, Datei geändert), wechselt das laufende
 * Gespräch die Rolle nicht: die Auswahl zeigt sie als „gilt ab dem nächsten Gespräch“.
 */
export const AssistentPicker: React.FC<{ gesperrt?: boolean }> = ({
  gesperrt = false,
}) => {
  const { t } = useTranslation();
  const assistenten = useAssistentStore((s) => s.assistenten);
  const switching = useVoiceCallStore((s) => s.switching);
  const [offen, setOffen] = useState(false);
  const [angesagt, setAngesagt] = useState("");
  const { ausstehend: spaeter } = useGeltenderAssistent();

  const aktiv = aktivesProfil(assistenten);
  const anzeige = (p: Profil) =>
    p.id === ALLES_ID ? t("voiceCall.assistant.all") : p.name;
  const name = anzeige(aktiv);
  // „Alles“ ist der Normalfall und braucht kein Schild: nur ein gewählter Assistent zeigt seinen Namen (der Platz im
  // Chatkopf ist knapp); der Name steht immer in der Beschriftung für den Screenreader.
  const gewaehlt = aktiv.id !== ALLES_ID;
  const ladefehler = assistenten.ladefehler !== null;
  // Ein Link statt des Ordners der Assistenten wird nie benutzt; der Text sagt es genauer als „nicht lesbar“.
  const ladeText =
    assistenten.ladefehler === "unsafe_path"
      ? "voiceCall.assistant.loadUnsafe"
      : "voiceCall.assistant.loadFailed";
  // Auf dem Telefon sind auch die Einträge des Menüs mindestens 44 px hoch.
  const eintrag = "max-sm:!min-h-[44px]";
  const vorlagen = VORLAGEN.filter((p) => p.id !== ALLES_ID);
  const eigene = assistenten.eigene.map((e) => e.profil);

  const waehlen = (id: string) => {
    if (id === aktiv.id) return;
    const profil =
      VORLAGEN.find((p) => p.id === id) ??
      eigene.find((p) => p.id === id) ??
      null;
    void useVoiceCallStore
      .getState()
      .assistentWechseln(id)
      .then((ok) => {
        if (ok && profil)
          setAngesagt(
            t("voiceCall.assistant.switched", { name: anzeige(profil) }),
          );
      });
  };

  return (
    <>
      <Menu
        open={offen && !gesperrt && !switching}
        onOpenChange={(_e, data) => setOffen(data.open)}
        checkedValues={{ assistent: [aktiv.id] }}
        onCheckedValueChange={(_e, data) => {
          const [id] = data.checkedItems;
          if (id) waehlen(id);
        }}
      >
        <MenuTrigger disableButtonEnhancement>
          <button
            type="button"
            data-testid="assistent-picker"
            aria-disabled={gesperrt || switching}
            aria-label={
              gesperrt
                ? t("voiceCall.assistant.ariaProjekt", { name })
                : spaeter
                  ? t("voiceCall.assistant.ariaSpaeter", { name })
                  : t("voiceCall.assistant.aria", { name })
            }
            title={
              gesperrt
                ? t("voiceCall.assistant.projekt")
                : spaeter
                  ? t("voiceCall.assistant.spaeter")
                  : t("voiceCall.assistant.title")
            }
            className={`inline-flex items-center gap-1 h-8 min-w-8 max-sm:h-[44px] max-sm:min-w-[44px] max-w-full rounded-full px-2 text-xs cursor-pointer hover:bg-mid-gray/15 border border-transparent focus-visible:border-logo-primary ${
              gewaehlt ? "" : "shrink-0"
            } ${gesperrt || switching ? "opacity-60" : ""}`}
          >
            <UserRound
              className={`w-3.5 h-3.5 shrink-0 ${
                gewaehlt ? "text-logo-primary" : "text-text-muted"
              }`}
              aria-hidden="true"
            />
            {gewaehlt && (
              <span className="truncate max-w-[10rem] max-sm:hidden font-semibold text-text/90">
                {name}
              </span>
            )}
            {spaeter && !gesperrt && (
              <Clock
                data-testid="assistent-spaeter"
                className="w-3 h-3 shrink-0 text-logo-primary"
                aria-hidden="true"
              />
            )}
            <ChevronDown
              className="w-3 h-3 shrink-0 text-text-muted"
              aria-hidden="true"
            />
          </button>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            <MenuGroup>
              <MenuGroupHeader>
                {t("voiceCall.assistant.groupAll")}
              </MenuGroupHeader>
              <MenuItemRadio
                name="assistent"
                value={ALLES_ID}
                disabled={switching}
                className={eintrag}
              >
                {t("voiceCall.assistant.all")}
              </MenuItemRadio>
            </MenuGroup>
            <MenuDivider />
            <MenuGroup>
              <MenuGroupHeader>
                {t("voiceCall.assistant.groupTemplates")}
              </MenuGroupHeader>
              {vorlagen.map((p) => (
                <MenuItemRadio
                  key={p.id}
                  name="assistent"
                  value={p.id}
                  disabled={switching || ladefehler}
                  className={eintrag}
                >
                  {p.name}
                </MenuItemRadio>
              ))}
            </MenuGroup>
            {eigene.length > 0 && (
              <>
                <MenuDivider />
                <MenuGroup>
                  <MenuGroupHeader>
                    {t("voiceCall.assistant.groupOwn")}
                  </MenuGroupHeader>
                  {eigene.map((p) => (
                    <MenuItemRadio
                      key={p.id}
                      name="assistent"
                      value={p.id}
                      disabled={switching}
                      className={eintrag}
                    >
                      {p.name}
                    </MenuItemRadio>
                  ))}
                </MenuGroup>
              </>
            )}
            {(ladefehler || assistenten.abgewiesen.length > 0) && (
              <>
                <MenuDivider />
                {ladefehler && (
                  <MenuItem
                    disabled
                    data-testid="assistent-ladefehler"
                    className={eintrag}
                  >
                    {t(ladeText)}
                  </MenuItem>
                )}
                {assistenten.abgewiesen.length > 0 && (
                  <MenuItem
                    disabled
                    data-testid="assistent-abgewiesen"
                    className={eintrag}
                  >
                    {t("voiceCall.assistant.rejected", {
                      count: assistenten.abgewiesen.length,
                    })}
                  </MenuItem>
                )}
              </>
            )}
          </MenuList>
        </MenuPopover>
      </Menu>
      <span role="status" aria-live="polite" className="sr-only">
        {angesagt}
      </span>
    </>
  );
};
