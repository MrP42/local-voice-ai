import {
  Button as FluentButton,
  type ButtonProps as FluentButtonProps,
  makeStyles,
  mergeClasses,
  Spinner,
  Tooltip,
  tokens,
} from "@fluentui/react-components";
import { forwardRef, type MouseEvent, type ReactNode } from "react";
import { cx } from "../cx";
import { useDichte } from "../dichte/Dichte";

/**
 * Hierarchie nach Fluent 2 / Windows 11:
 * - primary: Akzent, hoechstens EINER je Ansicht (die naechste sinnvolle Aktion)
 * - secondary: Standard (Default) — neutral mit Rahmen
 * - outline: transparent mit Rahmen, fuer Nebenaktionen auf farbigem Grund
 * - subtle: ohne Rahmen, Flaeche erst bei Hover — Werkzeugleisten, Listen, Kopfzeilen
 * - transparent: nur Text/Icon, fuer Inline-Aktionen
 * - danger: destruktiv und endgueltig (Loeschen), sparsam; sonst secondary + Bestaetigung
 * - ghost: Alias fuer subtle (Bestand)
 */
export type ButtonVariant =
  | "primary"
  | "secondary"
  | "outline"
  | "subtle"
  | "transparent"
  | "danger"
  | "ghost";
export type ButtonSize = "small" | "medium" | "large";

const APPEARANCE: Record<ButtonVariant, FluentButtonProps["appearance"]> = {
  primary: "primary",
  secondary: "secondary",
  outline: "outline",
  subtle: "subtle",
  transparent: "transparent",
  danger: "primary",
  ghost: "subtle",
};

const useStyles = makeStyles({
  danger: {
    backgroundColor: tokens.colorStatusDangerBackground3,
    color: tokens.colorNeutralForegroundOnBrand,
    ":hover": {
      backgroundColor: tokens.colorStatusDangerBackground3Hover,
      color: tokens.colorNeutralForegroundOnBrand,
    },
    ":hover:active": {
      backgroundColor: tokens.colorStatusDangerBackground3Pressed,
      color: tokens.colorNeutralForegroundOnBrand,
    },
  },
  // Gesperrt sieht „danger“ aus wie jede gesperrte Fluent-Schaltfläche (nicht rot und aktiv).
  dangerGesperrt: {
    backgroundColor: tokens.colorNeutralBackgroundDisabled,
    color: tokens.colorNeutralForegroundDisabled,
    ":hover": {
      backgroundColor: tokens.colorNeutralBackgroundDisabled,
      color: tokens.colorNeutralForegroundDisabled,
    },
    ":hover:active": {
      backgroundColor: tokens.colorNeutralBackgroundDisabled,
      color: tokens.colorNeutralForegroundDisabled,
    },
  },
  touch: {
    "@media (pointer: coarse)": { minHeight: "44px", minWidth: "44px" },
  },
});

/**
 * Fluent typisiert `ButtonProps` als Vereinigung aus `as="button"` und `as="a"`. `Omit` darueber
 * vermischt beide Zweige (Handler fuer Anker und Button), deshalb nur der Button-Zweig.
 */
type FluentNativeButtonProps = Extract<FluentButtonProps, { as?: "button" }>;

function stopActivation(e: MouseEvent<HTMLButtonElement>) {
  e.preventDefault();
}

export interface ButtonProps
  extends Omit<
    FluentNativeButtonProps,
    "appearance" | "disabled" | "onClick" | "size" | "children"
  > {
  variant?: ButtonVariant;
  size?: ButtonSize;
  children?: ReactNode;
  /** Ausloesen per Klick, Enter oder Leertaste (Bestands-API, entspricht onClick). */
  onPress?: (e: MouseEvent<HTMLButtonElement>) => void;
  onClick?: (e: MouseEvent<HTMLButtonElement>) => void;
  isDisabled?: boolean;
  /** Laeuft: Spinner statt Icon, aria-busy, weitere Ausloesungen blockiert (bleibt fokussierbar). */
  isPending?: boolean;
  /** Bestands-API (React Aria Slots); ohne Wirkung. */
  slot?: string;
  className?: string;
}

/**
 * Schaltflaeche auf Fluent 2. Beschriftung im Satzbau: Verb + Objekt, ohne Punkt („Entwurf speichern“).
 * Reicht den Ref an das `button` durch: Tooltip und MenuTrigger verankern sich daran (auch unter React 18).
 */
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = "secondary",
    size,
    onPress,
    onClick,
    isDisabled,
    isPending,
    slot: _slot,
    icon,
    className,
    children,
    ...props
  },
  ref,
) {
  const s = useStyles();
  const dichte = useDichte();
  const handler = onPress ?? onClick;
  return (
    <FluentButton
      {...props}
      ref={ref}
      appearance={APPEARANCE[variant]}
      size={size ?? (dichte === "kompakt" ? "small" : "medium")}
      disabled={isDisabled && !isPending}
      disabledFocusable={isPending}
      aria-busy={isPending || undefined}
      icon={isPending ? <Spinner size="extra-tiny" aria-label="Wird ausgeführt" /> : icon}
      onClick={isPending || isDisabled ? undefined : handler}
      // Fluent setzt bei disabledFocusable nur aria-disabled; ein type="submit" wuerde das Formular
      // trotzdem absenden (Klick und Enter im Feld). Die Aktivierung hier abbrechen.
      onClickCapture={isPending ? stopActivation : undefined}
      className={mergeClasses(
        cx("wai-btn", `wai-btn--${variant}`, className),
        variant === "danger" && s.danger,
        variant === "danger" && isDisabled && !isPending && s.dangerGesperrt,
        s.touch,
      )}
    >
      {children}
    </FluentButton>
  );
});

type FluentAnchorButtonProps = Extract<FluentButtonProps, { as: "a" }>;

export interface LinkButtonProps
  extends Omit<FluentAnchorButtonProps, "as" | "appearance" | "size" | "children" | "href"> {
  /** Ziel der Navigation (Pflicht: ein Link ohne Ziel ist eine Schaltflaeche). */
  href: string;
  variant?: Exclude<ButtonVariant, "danger">;
  size?: ButtonSize;
  children?: ReactNode;
  className?: string;
}

/**
 * Navigation im Aussehen einer Schaltflaeche: rendert ein echtes `a` (Mittelklick, neuer Tab,
 * funktioniert ohne JavaScript). Fuer Aktionen `Button` verwenden.
 */
export const LinkButton = forwardRef<HTMLAnchorElement, LinkButtonProps>(function LinkButton(
  { variant = "secondary", size, className, children, ...props },
  ref,
) {
  const s = useStyles();
  const dichte = useDichte();
  return (
    <FluentButton
      {...props}
      as="a"
      ref={ref}
      appearance={APPEARANCE[variant]}
      size={size ?? (dichte === "kompakt" ? "small" : "medium")}
      className={mergeClasses(cx("wai-btn", `wai-btn--${variant}`, className), s.touch)}
    >
      {children}
    </FluentButton>
  );
});

export interface IconButtonProps extends Omit<ButtonProps, "children" | "aria-label" | "icon"> {
  /** Pflicht: Name des Buttons, erscheint zugleich als Tooltip (Windows-Verhalten). */
  "aria-label": string;
  icon: ReactNode;
  /** Tooltip abschalten, wenn derselbe Text sichtbar daneben steht. */
  tooltip?: boolean;
}

/** Nur-Icon-Schaltflaeche; Standard subtle wie in Office-Kopf- und Werkzeugleisten. */
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { icon, variant = "subtle", tooltip = true, "aria-label": label, className, ...props },
  ref,
) {
  const btn = (
    <Button
      {...props}
      ref={ref}
      variant={variant}
      aria-label={label}
      icon={<span className="wai-btn__icon">{icon}</span>}
      className={cx("wai-btn--icon", className)}
    />
  );
  if (!tooltip) return btn;
  return (
    <Tooltip content={label} relationship="label" withArrow>
      {btn}
    </Tooltip>
  );
});
