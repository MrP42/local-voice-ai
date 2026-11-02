import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  FREIGABE_LABELS,
  PROJEKTORDNER_LABELS,
  RECHERCHE_LABELS,
  type ProjektordnerLabels,
  type RechercheAnsichtLabels,
  type RechercheFreigabeLabels,
} from "../../vendor/wai-ui/projekt";

/**
 * Beschriftungen der Bausteine aus @wai/ui (Projektordner, Recherche, Freigabe) aus den
 * Übersetzungen (`projekte.*`). Fehlt ein Schlüssel, gilt der deutsche Standardtext des Bausteins.
 */
const tr = (t: TFunction, key: string, def: string, opts?: object): string =>
  t(`projekte.${key}`, { defaultValue: def, ...opts }) as string;

function zuordnen<K extends string>(
  t: TFunction,
  prefix: string,
  defaults: Record<K, string>,
): Record<K, string> {
  const out = {} as Record<K, string>;
  for (const k of Object.keys(defaults) as K[]) {
    out[k] = tr(t, `${prefix}.${k}`, defaults[k]);
  }
  return out;
}

const einfach = <T extends object>(
  t: TFunction,
  prefix: string,
  defaults: T,
): Record<string, string> => {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(defaults)) {
    if (typeof v === "string") out[k] = tr(t, `${prefix}.${k}`, v);
  }
  return out;
};

export function useOrdnerLabels(): ProjektordnerLabels {
  const { t } = useTranslation();
  return useMemo(() => {
    const D = PROJEKTORDNER_LABELS;
    return {
      ...D,
      ...einfach(t, "ordner", D),
      entryCount: (n: number) =>
        n === 1
          ? tr(t, "ordner.entryCountOne", "1 Eintrag")
          : tr(t, "ordner.entryCountMany", `${n} Einträge`, { n }),
      readingProgress: (p: number) =>
        tr(t, "ordner.readingProgress", `Vorlesefortschritt ${p} %`, { p }),
      copyTargets: zuordnen(t, "ordner.copyTargets", D.copyTargets),
      groups: zuordnen(t, "ordner.groups", D.groups),
      classes: zuordnen(t, "klassen", D.classes),
      kinds: zuordnen(t, "ordner.kinds", D.kinds),
      apps: zuordnen(t, "ordner.apps", D.apps),
    };
  }, [t]);
}

export function useRechercheLabels(): RechercheAnsichtLabels {
  const { t } = useTranslation();
  return useMemo(() => {
    const D = RECHERCHE_LABELS;
    return {
      ...D,
      ...einfach(t, "recherche", D),
      inProject: (title: string) =>
        tr(t, "recherche.inProject", `im Projekt „${title}“`, { title }),
      effort: (a) =>
        tr(
          t,
          "recherche.effort",
          `ca. ${a.seconds} s · bis ${a.maxSources} Quellen · ${a.model}`,
          { seconds: a.seconds, maxSources: a.maxSources, model: a.model },
        ),
      contradictionRefs: (i) =>
        tr(t, "recherche.contradictionRefs", `(Quellen ${i.join(", ")})`, {
          list: i.join(", "),
        }),
      sources: (n) => tr(t, "recherche.sources", `Quellen · ${n}`, { n }),
      footnote: (n) => tr(t, "recherche.footnote", `Quelle ${n}`, { n }),
      highlighted: (n, title) =>
        tr(t, "recherche.highlighted", `Quelle ${n} hervorgehoben: ${title}`, {
          n,
          title,
        }),
      modes: zuordnen(t, "recherche.modes", D.modes),
      scopes: zuordnen(t, "recherche.scopes", D.scopes),
      actions: zuordnen(t, "recherche.actions", D.actions),
      sourceTypes: zuordnen(t, "recherche.sourceTypes", D.sourceTypes),
    };
  }, [t]);
}

export function useFreigabeLabels(): RechercheFreigabeLabels {
  const { t } = useTranslation();
  return useMemo(() => {
    const D = FREIGABE_LABELS;
    return {
      ...D,
      ...einfach(t, "freigabe", D),
      context: (l: string) =>
        tr(t, "freigabe.context", `Kontext: ${l}.`, { label: l }),
    };
  }, [t]);
}
