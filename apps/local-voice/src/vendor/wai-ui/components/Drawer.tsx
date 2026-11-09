import {
  DrawerBody,
  DrawerFooter,
  DrawerHeader,
  DrawerHeaderTitle,
  OverlayDrawer,
} from "@fluentui/react-components";
import { type ReactNode, useId } from "react";
import { Icon } from "../icons";
import { IconButton } from "./Button";
import { useFocusReturn } from "./Dialog";

export interface DrawerProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  children?: ReactNode;
  /** Fußzeile mit Aktionen (Speichern, Abbrechen). */
  footer?: ReactNode;
  /** Seite, von der das Panel einfährt; Standard rechts (`end`). */
  position?: "start" | "end";
  size?: "small" | "medium" | "large";
  /** Modal (Standard): Fokusfalle, Hintergrund inert. Nicht modal: Hintergrund bleibt bedienbar. */
  modal?: boolean;
  closeLabel?: string;
}

/**
 * Seitliches Panel auf Fluent 2. Escape und Schließen-Schaltfläche schließen; der Fokus kehrt zum
 * Auslöser zurück (bzw. in den Hauptbereich, wenn dieser nicht mehr existiert).
 */
export function Drawer({
  isOpen,
  onOpenChange,
  title,
  children,
  footer,
  position = "end",
  size = "medium",
  modal = true,
  closeLabel = "Schließen",
}: DrawerProps) {
  useFocusReturn(isOpen);
  const descId = useId();

  return (
    <OverlayDrawer
      open={isOpen}
      position={position}
      size={size}
      modalType={modal ? "modal" : "non-modal"}
      onOpenChange={(_e, data) => onOpenChange(data.open)}
      className="wai-drawer"
      aria-describedby={descId}
    >
      <DrawerHeader>
        <DrawerHeaderTitle
          action={
            <IconButton
              aria-label={closeLabel}
              icon={<Icon name="close" />}
              onPress={() => onOpenChange(false)}
            />
          }
        >
          {title}
        </DrawerHeaderTitle>
      </DrawerHeader>
      <DrawerBody id={descId}>{children}</DrawerBody>
      {footer && <DrawerFooter>{footer}</DrawerFooter>}
    </OverlayDrawer>
  );
}
