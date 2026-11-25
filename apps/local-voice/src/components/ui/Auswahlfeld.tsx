import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Option,
  OptionGroup,
  renderCombobox_unstable,
  renderDropdown_unstable,
  useCombobox_unstable,
  useComboboxContextValues,
  useComboboxStyles_unstable,
  useDropdown_unstable,
  useDropdownStyles_unstable,
} from "@fluentui/react-components";
import {
  filtere,
  sprungZiel,
  trefferTeile,
  type Bereich,
} from "./auswahlSuche";
import "./Auswahlfeld.css";

/**
 * Auswahlfelder mit Buchstabensprung und Teilwortsuche (lva#839), gebaut aus den
 * Bausteinen von Fluent `Dropdown` und `Combobox` (ARIA-Combobox-Muster: Auslöser
 * `role="combobox"` mit `aria-expanded`, `aria-controls` und
 * `aria-activedescendant`, Liste `role="listbox"`, Einträge `role="option"`).
 *
 * - `ListenAuswahl`: feste, kurze Liste. Ein Buchstabe springt zum nächsten Eintrag
 *   mit diesem Anfang (wiederholt: weiter), ä/ae usw. gelten als gleich.
 * - `SuchAuswahl`: freies Textfeld mit Vorschlägen. Jeder getippte Text, auch ein
 *   einzelner Buchstabe, filtert nach Teilwort; Anfangstreffer stehen vorn, die
 *   Fundstelle ist hervorgehoben. Der getippte Text ist der Wert; ein Vorschlag wird
 *   nur mit Pfeiltaste + Enter (oder Klick) übernommen.
 *
 * NVDA-Notiz: Fluent hält den Fokus im Auslöser und führt die aktive Option über
 * `aria-activedescendant`; NVDA liest beim Pfeilen die Option, beim Tippen im
 * Suchfeld die Zahl der Vorschläge (Statuszeile, höflich).
 */

export interface AuswahlOption {
  value: string;
  label: string;
  disabled?: boolean;
  /** Überschrift der Gruppe (z. B. „Mitgeliefert“, „Eigene“). Einträge einer
   *  Gruppe müssen in `options` aufeinander folgen. */
  gruppe?: string;
}

interface Gemeinsam {
  /** Kennung des Auslösers (für `<label htmlFor>`). */
  id?: string;
  /** Zugänglicher Name, wenn keine sichtbare Beschriftung verbunden ist. */
  ariaLabel?: string;
  disabled?: boolean;
  placeholder?: string;
  className?: string;
  "data-testid"?: string;
}

/** Zeit, nach der getippte Buchstaben eine neue Suche beginnen (wie Windows). */
const TIPPPAUSE_MS = 500;

/** Tasten, mit denen der Nutzer selbst einen Vorschlag ansteuert. */
const NAVIGATION = new Set([
  "ArrowDown",
  "ArrowUp",
  "Home",
  "End",
  "PageDown",
  "PageUp",
]);

/** Einträge, in Gruppen gefasst, wo `gruppe` gesetzt ist. */
function gruppiert<T>(
  eintraege: readonly T[],
  gruppe: (e: T) => string | undefined,
  zeile: (e: T, i: number) => React.ReactNode,
): React.ReactNode[] {
  const knoten: React.ReactNode[] = [];
  let i = 0;
  while (i < eintraege.length) {
    const name = gruppe(eintraege[i]);
    if (name === undefined) {
      knoten.push(zeile(eintraege[i], i));
      i++;
      continue;
    }
    const kinder: React.ReactNode[] = [];
    while (i < eintraege.length && gruppe(eintraege[i]) === name) {
      kinder.push(zeile(eintraege[i], i));
      i++;
    }
    knoten.push(
      <OptionGroup key={`gruppe-${name}-${i}`} label={name}>
        {kinder}
      </OptionGroup>,
    );
  }
  return knoten;
}

/** Beschriftung mit hervorgehobener Fundstelle. Ein Element: die Option ist ein
 *  Flex-Container und setzte sonst Abstände zwischen die Textstücke. */
const MitTreffer: React.FC<{ text: string; bereich: Bereich | null }> = ({
  text,
  bereich,
}) => (
  <span>
    {trefferTeile(text, bereich).map((teil, i) =>
      teil.treffer ? (
        <mark key={i} className="lv-auswahl__treffer">
          {teil.text}
        </mark>
      ) : (
        <React.Fragment key={i}>{teil.text}</React.Fragment>
      ),
    )}
  </span>
);

interface ListenAuswahlProps extends Gemeinsam {
  options: readonly AuswahlOption[];
  value: string | null;
  onChange: (value: string) => void;
}

/** Feste Auswahlliste (Fluent Dropdown) mit Buchstabensprung. */
export const ListenAuswahl: React.FC<ListenAuswahlProps> = ({
  options,
  value,
  onChange,
  id,
  ariaLabel,
  disabled,
  placeholder,
  className = "",
  "data-testid": testId,
}) => {
  const { t } = useTranslation();
  // Fluent sucht die aktive Option per CSS-Selektor: Kennung ohne die Doppelpunkte von useId.
  const basis = `lv-auswahl-${useId().replace(/[^\w-]/g, "")}`;
  const optionId = (i: number) => `${basis}-option-${i}`;
  const gewaehlt = options.find((o) => o.value === value);

  const state = useDropdown_unstable(
    {
      id,
      "aria-label": ariaLabel,
      disabled,
      placeholder,
      className: `lv-dropdown lv-auswahl ${className}`.trim(),
      value: gewaehlt?.label ?? "",
      selectedOptions: value !== null ? [value] : [],
      onOptionSelect: (_e, data) => {
        if (data.optionValue !== undefined) onChange(data.optionValue);
      },
      ...(testId ? { "data-testid": testId } : {}),
      children:
        options.length === 0 ? (
          <Option value="" disabled text={t("common.noOptionsFound")}>
            {t("common.noOptionsFound")}
          </Option>
        ) : (
          gruppiert(
            options,
            (o) => o.gruppe,
            (o, i) => (
              <Option
                key={o.value}
                id={optionId(i)}
                value={o.value}
                disabled={o.disabled}
                text={o.label}
              >
                {o.label}
              </Option>
            ),
          )
        ),
    },
    null,
  );

  const { activeDescendantController: steuerung, open, setOpen } = state;
  const getippt = useRef("");
  const pause = useRef<number | undefined>(undefined);
  // Sprungziel, wenn der Buchstabe die Liste erst öffnet: Fluent setzt beim Öffnen
  // die gewählte Option aktiv, der Sprung kommt danach (Effekt unten).
  const ziel = useRef<string | null>(null);

  useEffect(() => () => window.clearTimeout(pause.current), []);
  useEffect(() => {
    if (open && ziel.current) {
      steuerung.focus(ziel.current);
      ziel.current = null;
    }
  }, [open, steuerung]);

  // Fluents eigene Tippsuche (nur Anfang, ohne ä/ae, erst ab dem zweiten
  // Buchstaben) wird ersetzt; alle anderen Tasten bleiben bei Fluent.
  const fluentTasten = state.button.onKeyDown;
  state.button.onKeyDown = (ev: React.KeyboardEvent<HTMLButtonElement>) => {
    const zeichen =
      ev.key.length === 1 &&
      ev.key !== " " &&
      !ev.altKey &&
      !ev.ctrlKey &&
      !ev.metaKey;
    if (!zeichen) {
      fluentTasten?.(ev);
      return;
    }
    ev.preventDefault();
    window.clearTimeout(pause.current);
    getippt.current += ev.key;
    pause.current = window.setTimeout(() => {
      getippt.current = "";
    }, TIPPPAUSE_MS);
    const aktivId = open ? steuerung.active() : undefined;
    const aktiv = open
      ? options.findIndex((_, i) => optionId(i) === aktivId)
      : options.findIndex((o) => o.value === value);
    const treffer = sprungZiel(options, getippt.current, aktiv);
    if (open) {
      if (treffer !== -1) steuerung.focus(optionId(treffer));
      return;
    }
    ziel.current = treffer === -1 ? null : optionId(treffer);
    setOpen(ev, true);
  };

  const kontext = useComboboxContextValues(state);
  useDropdownStyles_unstable(state);
  return renderDropdown_unstable(state, kontext);
};

interface SuchAuswahlProps extends Gemeinsam {
  /** Vorschläge; der Wert ist trotzdem jeder Text. */
  vorschlaege: readonly AuswahlOption[];
  value: string;
  onChange: (text: string) => void;
}

/** Freies Textfeld mit Vorschlägen (Fluent Combobox, `freeform`) und Teilwortsuche. */
export const SuchAuswahl: React.FC<SuchAuswahlProps> = ({
  vorschlaege,
  value,
  onChange,
  id,
  ariaLabel,
  disabled,
  placeholder,
  className = "",
  "data-testid": testId,
}) => {
  const { t } = useTranslation();
  const [offen, setOffen] = useState(false);
  // `null`: keine Suche (Feld frisch betreten oder Vorschlag übernommen), alle Vorschläge.
  const [suchtext, setSuchtext] = useState<string | null>(null);
  const [fokus, setFokus] = useState(false);
  const treffer = filtere(vorschlaege, suchtext ?? "");
  const gewaehlt = vorschlaege.find((v) => v.label === value);

  const state = useCombobox_unstable(
    {
      id,
      "aria-label": ariaLabel,
      "aria-autocomplete": "list",
      disabled,
      placeholder,
      className: `lv-auswahl lv-auswahl--suche ${className}`.trim(),
      freeform: true,
      // Nichts vorauswählen: Enter behält sonst nicht den getippten Text.
      disableAutoFocus: true,
      value,
      open: offen && treffer.length > 0,
      onOpenChange: (_e, data) => {
        setOffen(data.open);
        if (!data.open) setSuchtext(null);
      },
      selectedOptions: gewaehlt ? [gewaehlt.value] : [],
      onOptionSelect: (_e, data) => {
        const option = vorschlaege.find((v) => v.value === data.optionValue);
        if (option) onChange(option.label);
        setSuchtext(null);
      },
      onChange: (ev: React.ChangeEvent<HTMLInputElement>) => {
        onChange(ev.target.value);
        setSuchtext(ev.target.value);
        if (vorschlaege.length > 0) setOffen(true);
      },
      onFocus: () => setFokus(true),
      onBlur: () => setFokus(false),
      expandIcon: vorschlaege.length > 0 ? undefined : null,
      ...(testId ? { "data-testid": testId } : {}),
      children: gruppiert(
        treffer,
        (tr) => tr.eintrag.gruppe,
        (tr) => (
          <Option
            key={tr.eintrag.value}
            value={tr.eintrag.value}
            disabled={tr.eintrag.disabled}
            text={tr.eintrag.label}
          >
            <MitTreffer text={tr.eintrag.label} bereich={tr.bereich} />
          </Option>
        ),
      ),
    },
    null,
  );

  // Fluent macht beim Tippen den ersten Vorschlag mit gleichem Anfang aktiv; Enter
  // oder Tab ersetzten dann „Ann“ durch „Anna …“. Aktiv ist ein Vorschlag hier erst,
  // wenn der Nutzer mit den Pfeiltasten (Pos1/Ende, Bild↑/↓) dorthin geht.
  const { activeDescendantController: steuerung } = state;
  const navigiert = useRef(false);
  useEffect(() => {
    navigiert.current = false;
    steuerung.blur();
  }, [suchtext, steuerung]);
  const fluentTasten = state.input.onKeyDown;
  state.input.onKeyDown = (ev: React.KeyboardEvent<HTMLInputElement>) => {
    if (NAVIGATION.has(ev.key)) navigiert.current = true;
    else if ((ev.key === "Enter" || ev.key === "Tab") && !navigiert.current)
      steuerung.blur();
    fluentTasten?.(ev);
  };

  const kontext = useComboboxContextValues(state);
  useComboboxStyles_unstable(state);
  const sucht = fokus && suchtext !== null && suchtext.trim() !== "";
  return (
    <>
      {renderCombobox_unstable(state, kontext)}
      <span className="sr-only" role="status" data-testid="auswahl-status">
        {sucht && vorschlaege.length > 0
          ? treffer.length > 0
            ? t("common.auswahlfeld.vorschlaege", { count: treffer.length })
            : t("common.auswahlfeld.keinVorschlag")
          : ""}
      </span>
    </>
  );
};
