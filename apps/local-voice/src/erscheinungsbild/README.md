# Erscheinungsbild (`@wai/ui erscheinungsbild v1`)

Standard-Baustein für alle WAI-Apps (ADR-0006, Nachtrag 17.10.2026): Windows-11-/Fluent-2-Aufbau, dazu wählbare
Designs mit Vorschaukarten, Akzentfarben, Farbschema, Kontrast und Animationen. Standard ist das Microsoft-Design
(Fluent `webLightTheme`/`webDarkTheme` unverändert); WAI ist optional.

## Inhalt

| Datei | Zweck |
|---|---|
| `designs.ts` | `AppDesign`, eingebaute Designs `MICROSOFT` (Standard) und `WAI`, Akzentliste `AKZENTE` |
| `farbe.ts` | Kontrast (WCAG), Akzentanpassung bis 4,5:1, `brandRampe` (Fluent-`BrandVariants` 10..160) |
| `theme.ts` | `themeFuerDesign(design, 'hell' \| 'dunkel', { akzent, kontrast })` → Fluent-`Theme` |
| `zustand.ts` | `useErscheinungsbild({ speicherSchluessel, designs })`: localStorage, `storage`-Ereignis, System-Schema, reduzierte Bewegung |
| `ErscheinungsbildProvider.tsx` | `FluentProvider` mit wirksamem Theme, `data-design`/`data-scheme` am Wurzel-Element |
| `DesignVorschau.tsx`, `DesignAuswahl.tsx`, `AkzentAuswahl.tsx` | Vorschaukarten (hell+dunkel), Radiogruppen mit Pfeiltasten |
| `ErscheinungsbildEinstellungen.tsx` | fertiger Abschnitt im Windows-11-Einstellungsstil, `EinstellungsKarte` für App-Zusätze |
| `Fensterknoepfe.tsx` | Windows-11-Fensterknöpfe mit injizierten Handlern (keine Tauri-Abhängigkeit) |
| `radiogruppe.tsx` | Tastaturmodell der Radiogruppen |

## Verwenden (Apps mit `@wai/ui`)

```tsx
import { ErscheinungsbildProvider, ErscheinungsbildEinstellungen } from "@wai/ui";

<ErscheinungsbildProvider speicherSchluessel="meine-app.erscheinungsbild.v1">
  <App />
</ErscheinungsbildProvider>

// Einstellungsseite
<ErscheinungsbildEinstellungen zusatz={<EinstellungsKarte kopf="Dichte" steuerung={…} />} />
```

Ohne Provider-Zustand: `const z = useErscheinungsbild({...})` und `<ErscheinungsbildProvider zustand={z}>`.
Texte überschreiben: `<ErscheinungsbildEinstellungen texte={{ titel: "Darstellung" }} />`.

## Kopieren (Apps ohne `@wai/ui`: Poststelle, Presenter, Local Voice)

1. Ordner `packages/ui/src/erscheinungsbild/` 1:1 nach `<app>/src/erscheinungsbild/` kopieren
   (Quelle: Worktree `D:/wt/wai-sdk/erscheinungsbild` bzw. Branch `wave/w1`). Der Test
   `erscheinungsbild.test.tsx` braucht `../test/axe` aus `@wai/ui`; mitkopieren oder die axe-Prüfung entfernen.
2. Abhängigkeiten: `react` ≥ 18, `@fluentui/react-components` 9.x. Nichts sonst (keine Icons-Bibliothek, kein Tailwind).
3. Wurzel der App in `<ErscheinungsbildProvider speicherSchluessel="<app>.erscheinungsbild.v1">` legen; eigenen
   `FluentProvider`/Theme-Schalter entfernen. App-CSS liest Farben über Fluent-Variablen (`var(--colorNeutralBackground1)`)
   oder `[data-scheme="dark"]` am Wurzel-Element.
4. Einstellungsseite: `<ErscheinungsbildEinstellungen />` einsetzen; app-eigene Zeilen über `zusatz`.
5. Rahmenlose Fenster (Tauri): `<Fensterknoepfe minimieren={() => w.minimize()} maximierenUmschalten={() => w.toggleMaximize()}
   schliessen={() => w.close()} istMaximiert={() => w.isMaximized()} beiGroessenaenderung={(cb) => w.onResized(cb)} />`
   mit `const w = getCurrentWindow()`.
6. Kopfkommentar `@wai/ui erscheinungsbild v1` stehen lassen; bei Updates den Ordner komplett ersetzen.

## Eigene Designs registrieren

```ts
import type { AppDesign } from "./erscheinungsbild";

export const NETZPULS: AppDesign = {
  id: "netzpuls",
  name: "Netzpuls",
  beschreibung: "Netz-Petrol, ruhig und nah an den Daten.",
  marke: { hell: "#003749", dunkel: "#68BDCC" },
  flaechen: {
    hell: { hintergrund: "#F2F6F7", unterflaeche: "#E7EEF0", karte: "#FFFFFF", navigation: "#E7EEF0" },
    dunkel: { hintergrund: "#0F1B1F", unterflaeche: "#0B1519", karte: "#122026", navigation: "#0B1519" },
  },
  radius: "standard",
};
const DESIGNS = [NETZPULS]; // stabile Referenz (Modulebene), nicht bei jedem Rendern neu bauen
<ErscheinungsbildProvider speicherSchluessel="…" designs={DESIGNS}>
```

- Reihenfolge der Karten: Microsoft, WAI, dann die App-Designs; gleiche `id` ersetzt ein eingebautes Design.
- `flaechen` weglassen → Fluent-Neutraltöne. Marke und Akzente werden automatisch auf 4,5:1 gegen alle vier Flächen
  gebracht; Schrift auf der Markenfläche ist Weiß oder Tinte, je nachdem, was besser lesbar ist.
- `markeAlsFlaecheDunkel: true` für helle Markenfarben im dunklen Schema (WAI-Signalgelb mit Tinte).
- Zuordnung zu Fluent: `karte` = `colorNeutralBackground1`, `hintergrund` = `…Background2`, `unterflaeche` =
  `…Background3`, `navigation` = `…Background4`.

## Regeln

- Microsoft-Design ohne Akzent ist **exakt** `webLightTheme`/`webDarkTheme` (Test). Keine gelben Flächen im Microsoft-Design.
- Kontrast „Hoch“ = `teamsHighContrastTheme`; Design und Akzent ruhen dann.
- Animationen aus (Schalter oder `prefers-reduced-motion`) → alle Fluent-Dauern 0 ms.
