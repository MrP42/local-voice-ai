import React from "react";
import { useTranslation } from "react-i18next";
import {
  Menu,
  MenuDivider,
  MenuGroup,
  MenuGroupHeader,
  MenuItem,
  MenuItemCheckbox,
  MenuItemRadio,
  MenuList,
  MenuPopover,
  MenuTrigger,
} from "@fluentui/react-components";
import { ChevronDown, Settings, Shield } from "lucide-react";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import {
  KNOWLEDGE_AREAS,
  SOUVERAENITAET_STUFEN,
  type KnowledgeArea,
} from "./orchestration";
import { useGeltenderAssistent } from "./profil/gilt";
import { wirksam } from "./profil/wirkung";
import { useKnowledgeEditor } from "./useKnowledge";

/**
 * Berechtigungs-Chip im Chatfeld (wie „Für mich genehmigen“ bei ChatGPT): zeigt die
 * Souveränitätsstufe des Gesprächs und die Zahl der freigegebenen Wissensbereiche.
 * Das Menü ändert genau dieselben Einstellungen wie die Reiter „Modelle“ und „Wissen“.
 * Die Zahl ist die der wirksamen Bereiche (lva#556): freigegeben UND vom gewählten Assistenten genutzt, also das, was die
 * KI im Gespräch tatsächlich sehen kann. Der Assistent öffnet nie mehr, als die Freigabe erlaubt; was er auslässt, steht im
 * Menü an dem Bereich und im Hinweis des Chips, getrennt von der Freigabe.
 */
export const PermissionChip: React.FC = () => {
  const { t } = useTranslation();
  const stufe = useVoiceCallStore((s) => s.config.orchestration.souveraenitaet);
  const setConfig = useVoiceCallStore((s) => s.setConfig);
  const openSettings = useVoiceCallStore((s) => s.openSettings);
  const { knowledge, toggle } = useKnowledgeEditor();
  const level = t(`voiceCall.settings.sovereigntyLevels.${stufe}`);
  const { wirkung } = useGeltenderAssistent();
  const genutzt = wirksam(wirkung, knowledge.areas);
  const genutztSet = new Set(genutzt);
  const count = genutzt.length;
  // Der Assistent lässt freigegebene Bereiche aus: dann erklärt der Hinweis Freigabe und Begrenzung getrennt.
  const begrenzt = wirkung !== null && count !== knowledge.areas.length;
  const hinweis = begrenzt
    ? t("voiceCall.permission.titleAssistent", {
        name: wirkung.profil.name,
        frei: knowledge.areas.length,
        count,
      })
    : null;
  const label =
    count === 0
      ? t("voiceCall.permission.chipNone", { level })
      : t("voiceCall.permission.chipCount", { level, count });

  return (
    <Menu
      checkedValues={{ level: [stufe], area: knowledge.areas }}
      onCheckedValueChange={(_e, data) => {
        if (data.name === "level") {
          const next = SOUVERAENITAET_STUFEN.find(
            (k) => k === data.checkedItems[0],
          );
          if (next)
            setConfig((c) => ({
              ...c,
              orchestration: { ...c.orchestration, souveraenitaet: next },
            }));
          return;
        }
        const on = new Set(data.checkedItems);
        for (const area of KNOWLEDGE_AREAS)
          if (on.has(area) !== knowledge.areas.includes(area))
            toggle(area as KnowledgeArea, on.has(area));
      }}
    >
      <MenuTrigger disableButtonEnhancement>
        <button
          type="button"
          data-testid="permission-chip"
          aria-label={
            hinweis
              ? `${t("voiceCall.permission.aria", { label })}. ${hinweis}`
              : t("voiceCall.permission.aria", { label })
          }
          title={hinweis ?? t("voiceCall.permission.title")}
          className="inline-flex items-center gap-1.5 h-8 min-w-8 max-sm:h-[44px] max-sm:min-w-[44px] max-w-full rounded-full px-2 text-xs cursor-pointer text-text/80 hover:bg-mid-gray/15 border border-transparent focus-visible:border-logo-primary"
        >
          <Shield className="w-4 h-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{label}</span>
          <ChevronDown
            className="w-3 h-3 shrink-0 text-text-muted"
            aria-hidden="true"
          />
        </button>
      </MenuTrigger>
      <MenuPopover>
        <MenuList>
          <MenuGroup>
            <MenuGroupHeader>{t("voiceCall.permission.level")}</MenuGroupHeader>
            {SOUVERAENITAET_STUFEN.map((k) => (
              <MenuItemRadio key={k} name="level" value={k} persistOnClick>
                {t(`voiceCall.settings.sovereigntyLevels.${k}`)}
              </MenuItemRadio>
            ))}
          </MenuGroup>
          <MenuDivider />
          <MenuGroup>
            <MenuGroupHeader>
              {t("voiceCall.permission.knowledge")}
            </MenuGroupHeader>
            {KNOWLEDGE_AREAS.map((area) => (
              <MenuItemCheckbox
                key={area}
                name="area"
                value={area}
                persistOnClick
                secondaryContent={
                  wirkung &&
                  knowledge.areas.includes(area) &&
                  !genutztSet.has(area)
                    ? t("voiceCall.permission.vomAssistentAusgelassen")
                    : undefined
                }
              >
                {t(`voiceCall.knowledge.areas.${area}.label`)}
              </MenuItemCheckbox>
            ))}
          </MenuGroup>
          <MenuDivider />
          <MenuItem
            icon={<Settings size={16} aria-hidden="true" />}
            onClick={() => openSettings("knowledge")}
          >
            {t("voiceCall.permission.allSettings")}
          </MenuItem>
        </MenuList>
      </MenuPopover>
    </Menu>
  );
};
