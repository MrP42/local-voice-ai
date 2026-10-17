/**
 * @wai/ui erscheinungsbild v1.1 — ErscheinungsbildProvider.tsx
 * Quelle: WAI-SDK packages/ui/src/erscheinungsbild.
 * Selbsttragend: importiert nur react, @fluentui/react-components und Nachbardateien.
 *
 * Wurzel einer App: FluentProvider mit dem wirksamen Theme. Das Wurzel-Element traegt `data-design`,
 * `data-scheme` (light|dark), `data-kontrast` und `data-animationen` fuer App-eigenes CSS.
 */
import { FluentProvider } from "@fluentui/react-components";
import { type CSSProperties, createContext, type ReactNode, useContext } from "react";
import {
  type Erscheinungsbild,
  type ErscheinungsbildOptionen,
  useErscheinungsbild,
} from "./zustand";

const Kontext = createContext<Erscheinungsbild | null>(null);

/** Zustand aus dem naechsten `ErscheinungsbildProvider`; `null` ausserhalb. */
export function useErscheinungsbildKontext(): Erscheinungsbild | null {
  return useContext(Kontext);
}

export interface ErscheinungsbildProviderProps extends ErscheinungsbildOptionen {
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  /** Fremd verwalteter Zustand (statt des eingebauten `useErscheinungsbild`). */
  zustand?: Erscheinungsbild;
}

function Wurzel({
  zustand,
  children,
  className,
  style,
}: {
  zustand: Erscheinungsbild;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <Kontext.Provider value={zustand}>
      <FluentProvider
        theme={zustand.theme}
        className={className}
        data-design={zustand.design.id}
        data-scheme={zustand.schema === "dunkel" ? "dark" : "light"}
        data-kontrast={zustand.kontrast}
        data-animationen={zustand.animationenWirksam ? "an" : "aus"}
        style={{ colorScheme: zustand.schema === "dunkel" ? "dark" : "light", ...style }}
      >
        {children}
      </FluentProvider>
    </Kontext.Provider>
  );
}

function Eigener(props: Omit<ErscheinungsbildProviderProps, "zustand">) {
  const { children, className, style, ...optionen } = props;
  const zustand = useErscheinungsbild(optionen);
  return (
    <Wurzel zustand={zustand} className={className} style={style}>
      {children}
    </Wurzel>
  );
}

export function ErscheinungsbildProvider({ zustand, ...rest }: ErscheinungsbildProviderProps) {
  if (zustand) {
    return (
      <Wurzel zustand={zustand} className={rest.className} style={rest.style}>
        {rest.children}
      </Wurzel>
    );
  }
  return <Eigener {...rest} />;
}
