import { useEffect, useRef, type KeyboardEvent, type ReactNode } from "react";
import { Check, Settings } from "lucide-react";

export type VoiceListOption = {
  value: string;
  label: string;
  isDisabled?: boolean;
};

export type VoiceListGroup = {
  key: string;
  title: string;
  items: { option: VoiceListOption; name: string; language: string | null }[];
};

/**
 * Teilt die Stimmen nach Anbieter auf (wie die iOS-Stimmenwahl):
 * „Phonon · Harper · Englisch“ → Gruppe „Phonon“, Name „Harper“, Sprache „Englisch“.
 */
export function groupVoices(
  options: VoiceListOption[],
  languageOf: (value: string) => string | null,
  langName: (code: string) => string,
  fishTitle: string,
): VoiceListGroup[] {
  const groups: VoiceListGroup[] = [];
  const byKey = new Map<string, VoiceListGroup>();
  for (const option of options) {
    let key: string;
    let title: string;
    let name = option.label;
    if (option.value.startsWith("provider:")) {
      key = `provider:${option.value.split(":")[1] ?? ""}`;
      const cut = option.label.indexOf(" · ");
      title = cut > 0 ? option.label.slice(0, cut) : option.label;
      name = cut > 0 ? option.label.slice(cut + 3) : option.label;
    } else if (option.value.startsWith("piper:")) {
      key = "piper";
      title = "Piper";
      name = option.label.replace(/ · Piper(?= – |$)/, "");
    } else {
      key = "fish";
      title = fishTitle;
    }
    const code = languageOf(option.value);
    const language = code ? langName(code) : null;
    if (language && name.endsWith(` · ${language}`))
      name = name.slice(0, -(language.length + 3));
    let group = byKey.get(key);
    if (!group) {
      group = { key, title, items: [] };
      byKey.set(key, group);
      groups.push(group);
    }
    group.items.push({ option, name, language });
  }
  return groups;
}

/**
 * Stimmenliste im Fenster der Leseleiste (Patrick 27.11.): eine Liste statt
 * Auswahlfeld mit aufgeklappter Liste darüber, nach Anbieter gruppiert, Name
 * und Sprache getrennt, die gewählte mit Haken. Pfeiltasten wandern durch die
 * Einträge, der Fokus startet auf der gewählten Stimme.
 */
export function ReaderVoiceList({
  groups,
  value,
  header,
  sample,
  manageLabel,
  label,
  onPick,
  onManage,
}: {
  groups: VoiceListGroup[];
  value: string;
  header?: ReactNode;
  sample: (option: VoiceListOption) => ReactNode;
  manageLabel: string;
  label: string;
  onPick: (value: string) => void;
  onManage: () => void;
}) {
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const root = listRef.current;
    if (!root) return;
    const current =
      root.querySelector<HTMLButtonElement>(
        '[aria-checked="true"]:not(:disabled)',
      ) ??
      root.querySelector<HTMLButtonElement>("[data-voice-item]:not(:disabled)");
    current?.focus({ preventScroll: true });
    current?.scrollIntoView({ block: "nearest" });
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    const root = listRef.current;
    if (!root) return;
    const items = [
      ...root.querySelectorAll<HTMLButtonElement>(
        "[data-voice-item]:not(:disabled)",
      ),
    ];
    if (!items.length) return;
    event.preventDefault();
    const at = items.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? items.length - 1
          : event.key === "ArrowDown"
            ? Math.min(items.length - 1, at + 1)
            : Math.max(0, at < 0 ? 0 : at - 1);
    items[next]?.focus();
    items[next]?.scrollIntoView({ block: "nearest" });
  };

  return (
    <div className="reader-voices" data-testid="reader-voice-list">
      {header && <div className="reader-voices__head">{header}</div>}
      <div
        ref={listRef}
        role="radiogroup"
        aria-label={label}
        className="reader-voices__list"
        onKeyDown={onKeyDown}
      >
        {groups.map((group) => (
          <div key={group.key} role="group" aria-label={group.title}>
            <div className="reader-voices__group" aria-hidden="true">
              {group.title}
            </div>
            {group.items.map(({ option, name, language }) => {
              const checked = option.value === value;
              return (
                <div key={option.value} className="reader-voices__row">
                  <button
                    type="button"
                    role="radio"
                    aria-checked={checked}
                    disabled={option.isDisabled}
                    className="reader-voices__item"
                    data-voice-item=""
                    data-testid={`reader-voice-${option.value}`}
                    onClick={() => onPick(option.value)}
                  >
                    <span className="reader-voices__check" aria-hidden="true">
                      {checked && <Check width={14} height={14} />}
                    </span>
                    <span className="reader-voices__name">{name}</span>
                    {language && (
                      <span className="reader-voices__lang">{language}</span>
                    )}
                  </button>
                  {!option.isDisabled && sample(option)}
                </div>
              );
            })}
          </div>
        ))}
      </div>
      <button
        type="button"
        className="reader-popover__item reader-voices__manage"
        onClick={onManage}
        data-testid="reader-voice-manage"
      >
        <Settings width={14} height={14} aria-hidden="true" />
        {manageLabel}
      </button>
    </div>
  );
}
