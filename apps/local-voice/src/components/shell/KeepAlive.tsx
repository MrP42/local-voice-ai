import React, {
  createContext,
  useContext,
  useLayoutEffect,
  useState,
} from "react";
import {
  HelpDockSlotContext,
  PageHeaderSlotContext,
  TitleContextSlotContext,
  useHelpDockSlot,
  usePageHeaderSlot,
  useTitleContextSlot,
} from "./PageHeaderSlot";

/**
 * Ist das Modul gerade sichtbar? Module, die beim Seitenwechsel erhalten
 * bleiben (Vorlesen, Bibliothek), hängen globale Tasten, die Seitenhilfe und
 * Aufträge aus anderen Modulen nur im sichtbaren Zustand ein. Außerhalb von
 * `KeepAlive` immer `true`.
 */
const SectionActiveContext = createContext(true);

export const useSectionActive = () => useContext(SectionActiveContext);

/** Eigener, dauerhafter Andockplatz, der nur im sichtbaren Zustand im echten
 *  Platz hängt: Portale behalten ihren Container und damit ihren Zustand. */
function useOwnSlot(target: HTMLElement | null, active: boolean) {
  const [own] = useState(() => {
    const el = document.createElement("div");
    el.className = "lv-keepslot";
    return el;
  });
  useLayoutEffect(() => {
    if (!target || !active) return;
    target.appendChild(own);
    return () => {
      own.remove();
    };
  }, [target, active, own]);
  return own;
}

/**
 * Hält ein Modul beim Wechsel in einen anderen Bereich am Leben (Patrick
 * 03.11.): Zustand, geladene Seite und Bildlaufstelle bleiben, beim Zurückkehren
 * erscheint es ohne Neuladen und ohne Aufblinken. Unsichtbar trägt es
 * `display: none`; seine Bedienelemente in der Titelleiste und die Seitenhilfe
 * liegen dann in einem abgehängten Platz.
 */
export const KeepAlive: React.FC<{
  active: boolean;
  name: string;
  children: React.ReactNode;
}> = ({ active, name, children }) => {
  const headerSlot = usePageHeaderSlot();
  const helpDock = useHelpDockSlot();
  const ownHeader = useOwnSlot(headerSlot, active);
  const ownDock = useOwnSlot(helpDock, active);
  const kontext = useTitleContextSlot();
  const ownKontext = useOwnSlot(kontext, active);
  return (
    <div
      className="lv-keepalive"
      data-keepalive={name}
      hidden={!active}
      style={{ display: active ? "contents" : "none" }}
    >
      <SectionActiveContext.Provider value={active}>
        <PageHeaderSlotContext.Provider value={headerSlot ? ownHeader : null}>
          <HelpDockSlotContext.Provider value={helpDock ? ownDock : null}>
            <TitleContextSlotContext.Provider
              value={kontext ? ownKontext : null}
            >
              {children}
            </TitleContextSlotContext.Provider>
          </HelpDockSlotContext.Provider>
        </PageHeaderSlotContext.Provider>
      </SectionActiveContext.Provider>
    </div>
  );
};
