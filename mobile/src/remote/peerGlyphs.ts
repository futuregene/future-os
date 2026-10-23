import type { PeerIconId } from "@future-os/peer-icons";
import type { LucideIcon } from "lucide-react-native";
import {
  Apple,
  AppWindow,
  Bot,
  Brain,
  Building2,
  ChartBar,
  Clapperboard,
  Cloud,
  Coffee,
  Cog,
  FlaskConical,
  Gamepad2,
  Globe,
  GraduationCap,
  HardDrive,
  House,
  Laptop,
  Lock,
  Microscope,
  Monitor,
  Palette,
  Rocket,
  Server,
  Shield,
  Smartphone,
  SquareTerminal,
  Star,
  Tv,
  Zap,
} from "lucide-react-native";

/**
 * How the phone draws each entry of the shared peer-icon set.
 *
 * The ids and their order live in `@future-os/peer-icons`, which the desktop
 * reads too; this table is only the phone's half — which line icon stands for
 * each id. `Record<PeerIconId, …>` rather than a literal object, so adding an
 * id to the shared set fails this build until it is drawn here, instead of
 * silently falling back to the default. The desktop has the matching table in
 * `desktop/src/features/remote-peer/peerGlyphs.ts`.
 *
 * Data, not a component: the glyph itself is `./PeerIconGlyph`, because a
 * module that exports a component should export nothing else.
 */
export const PEER_GLYPHS: Record<PeerIconId, LucideIcon> = {
  desktop: Monitor,
  laptop: Laptop,
  phone: Smartphone,
  server: Server,
  cloud: Cloud,
  windows: AppWindow,
  macos: Apple,
  linux: SquareTerminal,
  home: House,
  office: Building2,
  cafe: Coffee,
  school: GraduationCap,
  rocket: Rocket,
  flask: FlaskConical,
  dev: Cog,
  build: Bot,
  data: ChartBar,
  storage: HardDrive,
  media: Tv,
  design: Palette,
  ai: Brain,
  research: Microscope,
  secure: Shield,
  private: Lock,
  fast: Zap,
  main: Star,
  studio: Clapperboard,
  gaming: Gamepad2,
  remote: Globe,
};
