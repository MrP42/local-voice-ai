import {
  Badge as FluentBadge,
  type BadgeProps as FluentBadgeProps,
} from "@fluentui/react-components";
import type { ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "../icons";

export type StatusTone = "neutral" | "info" | "success" | "warning" | "danger";

const COLOR: Record<StatusTone, FluentBadgeProps["color"]> = {
  neutral: "informative",
  info: "brand",
  success: "success",
  warning: "warning",
  danger: "danger",
};
const ICON: Record<StatusTone, IconName> = {
  neutral: "dot",
  info: "info",
  success: "success",
  warning: "warning",
  danger: "error",
};

export interface BadgeProps {
  tone?: StatusTone;
  children: ReactNode;
  className?: string;
}

/** Reine Label-/Zaehlmarke ohne Statussemantik (Fluent Badge, getoent). */
export function Badge({ tone = "neutral", children, className }: BadgeProps) {
  return (
    <FluentBadge
      appearance="tint"
      color={COLOR[tone]}
      shape="rounded"
      className={cx("wai-badge", `wai-badge--${tone}`, className)}
    >
      <span>{children}</span>
    </FluentBadge>
  );
}

/** Status nie nur ueber Farbe: Symbol plus Text. */
export function StatusChip({ tone = "neutral", children, className }: BadgeProps) {
  return (
    <FluentBadge
      appearance="tint"
      color={COLOR[tone]}
      shape="rounded"
      icon={<Icon name={ICON[tone]} />}
      className={cx("wai-badge", "wai-chip", `wai-badge--${tone}`, className)}
    >
      <span>{children}</span>
    </FluentBadge>
  );
}
