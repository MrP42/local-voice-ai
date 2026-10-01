import React, { useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type PersonSummary } from "@/bindings";
import { Input } from "../../ui/Input";

/** Höchstens so viele Personen unter dem Feld. */
const MAX_MATCHES = 5;

/** Vergleichsform: klein, ohne Akzente ("André" findet "andre"). */
const fold = (value: string) =>
  value.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase().trim();

/** Die bekannten Personen (P5d) einmal je geöffnetem Feld holen. */
const usePeople = () => {
  const [people, setPeople] = useState<PersonSummary[]>([]);
  useEffect(() => {
    let cancelled = false;
    void commands
      .peopleList(null)
      .then((result) => {
        if (cancelled) return;
        if (result.status === "ok" && Array.isArray(result.data)) {
          setPeople(result.data);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);
  return people;
};

interface PersonNameFieldProps {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  /** Eine bekannte Person wurde gewählt (Klick auf den Treffer). */
  onPick: (person: PersonSummary) => void;
  placeholder: string;
  ariaLabel?: string;
  autoFocus?: boolean;
  inputTestId?: string;
  disabled?: boolean;
  className?: string;
  /** Namen, die nicht angeboten werden (z. B. der aktuelle Name des Sprechers). */
  exclude?: string[];
}

/**
 * Namensfeld mit Autovervollständigung aus den bekannten Personen: dieselbe
 * Person bekommt so in jeder Besprechung dieselbe Kennung (gleicher Name =
 * gleiche Person), und niemand tippt "Anna Berg" einmal als "Berg, Anna".
 * Ein freier Name bleibt möglich; Enter im Feld gehört dem Formular.
 */
export const PersonNameField: React.FC<PersonNameFieldProps> = ({
  id,
  value,
  onChange,
  onPick,
  placeholder,
  ariaLabel,
  autoFocus,
  inputTestId,
  disabled,
  className = "",
  exclude = [],
}) => {
  const { t } = useTranslation();
  const listId = useId();
  const people = usePeople();
  const query = fold(value);
  const skip = useMemo(() => new Set(exclude.map(fold)), [exclude]);

  const matches = useMemo(() => {
    if (query === "") return [];
    return people
      .filter((p) => {
        const name = fold(p.name);
        return name.includes(query) && name !== query && !skip.has(name);
      })
      .slice(0, MAX_MATCHES);
  }, [people, query, skip]);

  return (
    <div className={`space-y-1 ${className}`}>
      <Input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={ariaLabel}
        aria-controls={matches.length > 0 ? listId : undefined}
        maxLength={80}
        autoFocus={autoFocus}
        disabled={disabled}
        autoComplete="off"
        data-testid={inputTestId}
        className="w-full min-w-0"
      />
      {matches.length > 0 && (
        <ul
          id={listId}
          aria-label={t("meetings.speakers.knownPeople")}
          data-testid="person-matches"
          className="overflow-hidden rounded-md border border-mid-gray/30"
        >
          {matches.map((person) => (
            <li key={person.id}>
              <button
                type="button"
                disabled={disabled}
                onClick={() => onPick(person)}
                data-testid="person-match"
                className="flex min-h-[32px] w-full cursor-pointer items-baseline justify-between gap-2 px-2 text-start text-sm hover:bg-mid-gray/15"
              >
                <span className="min-w-0 truncate">{person.name}</span>
                {person.is_self && (
                  <span className="shrink-0 text-xs text-text/50">
                    {t("meetings.speakers.self")}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
