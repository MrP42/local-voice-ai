import {
  Camera,
  ChevronRight,
  LogIn,
  LogOut,
  Lock,
  type LucideProps,
  MonitorSmartphone,
  Plus,
  ShieldCheck,
  User,
} from "lucide-react";
import type { ComponentType } from "react";

/** Ersatz für `Icon` aus @wai/ui: dieselben Namen, Motive aus lucide-react. */
const ICONS = {
  user: User,
  logIn: LogIn,
  logOut: LogOut,
  lock: Lock,
  plus: Plus,
  camera: Camera,
  shield: ShieldCheck,
  devices: MonitorSmartphone,
  chevronRight: ChevronRight,
} satisfies Record<string, ComponentType<LucideProps>>;

export type IconName = keyof typeof ICONS;

export function Icon({
  name,
  ...rest
}: { name: IconName } & LucideProps): React.ReactElement {
  const Motiv = ICONS[name];
  return <Motiv size="1em" aria-hidden="true" focusable="false" {...rest} />;
}
