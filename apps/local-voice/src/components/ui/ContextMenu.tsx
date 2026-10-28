import React, { useCallback, useMemo, useState } from "react";
import {
  Menu,
  MenuDivider,
  MenuItem,
  MenuList,
  MenuPopover,
  tokens,
} from "@fluentui/react-components";
import type { LucideIcon } from "lucide-react";

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: LucideIcon;
  onSelect: () => void;
  disabled?: boolean;
  /** Zerstoerende Aktion: rot. */
  danger?: boolean;
  testId?: string;
  /** Trennlinie vor dem Eintrag. */
  separatorBefore?: boolean;
}

interface OpenState {
  x: number;
  y: number;
  items: ContextMenuItem[];
  label: string;
}

/**
 * Rechtsklick-Menue im Fluent-Stil (Fluent `Menu` an der Mausposition) -- eine
 * Quelle fuer Seitenliste, Stimmenwahl und Leisten, statt des Browsermenues.
 *
 * `open(event, items)` im `onContextMenu` der Flaeche; `menu` einmal irgendwo im
 * Baum rendern. Die Tastatur-Menuetaste oeffnet neben dem Element selbst.
 */
export function useContextMenu(menuLabel: string) {
  const [state, setState] = useState<OpenState | null>(null);

  const open = useCallback(
    (event: React.MouseEvent, items: ContextMenuItem[]) => {
      event.preventDefault();
      event.stopPropagation();
      let { clientX: x, clientY: y } = event;
      if (x === 0 && y === 0) {
        const r = (event.currentTarget as HTMLElement).getBoundingClientRect();
        x = r.left + 16;
        y = r.top + 16;
      }
      setState({ x, y, items, label: menuLabel });
    },
    [menuLabel],
  );

  const close = useCallback(() => setState(null), []);

  const menu = useMemo(() => {
    if (!state) return null;
    const { x, y, items, label } = state;
    return (
      <Menu
        open
        onOpenChange={(_event, data) => {
          if (!data.open) close();
        }}
        positioning={{
          target: { getBoundingClientRect: () => new DOMRect(x, y, 0, 0) },
          position: "below",
          align: "start",
          fallbackPositions: ["above", "after", "before"],
        }}
      >
        <MenuPopover data-testid="context-menu">
          <MenuList aria-label={label}>
            {items.map((item) => {
              const Icon = item.icon;
              return (
                <React.Fragment key={item.id}>
                  {item.separatorBefore && <MenuDivider />}
                  <MenuItem
                    icon={
                      Icon ? <Icon size={16} aria-hidden="true" /> : undefined
                    }
                    disabled={item.disabled}
                    data-testid={item.testId}
                    style={
                      item.danger
                        ? { color: tokens.colorPaletteRedForeground1 }
                        : undefined
                    }
                    onClick={() => {
                      close();
                      item.onSelect();
                    }}
                  >
                    {item.label}
                  </MenuItem>
                </React.Fragment>
              );
            })}
          </MenuList>
        </MenuPopover>
      </Menu>
    );
  }, [state, close]);

  return { open, menu };
}
