// The app's icons: one semantic name each, drawn by Lucide (lucide-solid,
// ISC licence; imported one file per icon so only these are bundled) at the
// device icons' stroke (DeviceIcon.tsx: 24-unit grid, 1.8, round caps, the
// current text colour). Decorative by default: whatever the icon sits in
// always has words or an aria-label. Every icon here is Lucide's; none is
// hand-drawn (none was missing).

import type { Component } from "solid-js";
import type { LucideProps } from "lucide-solid";
import ActivityIcon from "lucide-solid/icons/activity";
import ChevronDownIcon from "lucide-solid/icons/chevron-down";
import ChevronRightIcon from "lucide-solid/icons/chevron-right";
import CircleCheckIcon from "lucide-solid/icons/circle-check";
import CircleXIcon from "lucide-solid/icons/circle-x";
import CpuIcon from "lucide-solid/icons/cpu";
import HardDriveIcon from "lucide-solid/icons/hard-drive";
import MemoryStickIcon from "lucide-solid/icons/memory-stick";
import WrenchIcon from "lucide-solid/icons/wrench";
import MousePointer2Icon from "lucide-solid/icons/mouse-pointer-2";
import PhoneCallIcon from "lucide-solid/icons/phone-call";
import ShieldHalfIcon from "lucide-solid/icons/shield-half";
import ArrowRightIcon from "lucide-solid/icons/arrow-right";
import BellDotIcon from "lucide-solid/icons/bell-dot";
import AppWindowIcon from "lucide-solid/icons/app-window";
import ArchiveIcon from "lucide-solid/icons/archive";
import ArrowUpDownIcon from "lucide-solid/icons/arrow-up-down";
import AudioLinesIcon from "lucide-solid/icons/audio-lines";
import BatteryMediumIcon from "lucide-solid/icons/battery-medium";
import BellIcon from "lucide-solid/icons/bell";
import BellRingIcon from "lucide-solid/icons/bell-ring";
import BluetoothIcon from "lucide-solid/icons/bluetooth";
import BookmarkIcon from "lucide-solid/icons/bookmark";
import CalendarClockIcon from "lucide-solid/icons/calendar-clock";
import CameraIcon from "lucide-solid/icons/camera";
import CheckIcon from "lucide-solid/icons/check";
import CircleOffIcon from "lucide-solid/icons/circle-off";
import CirclePlayIcon from "lucide-solid/icons/circle-play";
import CircleQuestionMarkIcon from "lucide-solid/icons/circle-question-mark";
import ClipboardIcon from "lucide-solid/icons/clipboard";
import ClipboardCopyIcon from "lucide-solid/icons/clipboard-copy";
import ClipboardPasteIcon from "lucide-solid/icons/clipboard-paste";
import ClockIcon from "lucide-solid/icons/clock";
import CodeIcon from "lucide-solid/icons/code";
import DownloadIcon from "lucide-solid/icons/download";
import EllipsisIcon from "lucide-solid/icons/ellipsis";
import FileUpIcon from "lucide-solid/icons/file-up";
import FolderIcon from "lucide-solid/icons/folder";
import FolderOpenIcon from "lucide-solid/icons/folder-open";
import FolderSyncIcon from "lucide-solid/icons/folder-sync";
import FolderUpIcon from "lucide-solid/icons/folder-up";
import GlobeIcon from "lucide-solid/icons/globe";
import HeartPulseIcon from "lucide-solid/icons/heart-pulse";
import ImageIcon from "lucide-solid/icons/image";
import InfoIcon from "lucide-solid/icons/info";
import KeyIcon from "lucide-solid/icons/key";
import KeyboardIcon from "lucide-solid/icons/keyboard";
import LanguagesIcon from "lucide-solid/icons/languages";
import LayoutDashboardIcon from "lucide-solid/icons/layout-dashboard";
import LayoutGridIcon from "lucide-solid/icons/layout-grid";
import LinkIcon from "lucide-solid/icons/link";
import LoaderIcon from "lucide-solid/icons/loader";
import LockIcon from "lucide-solid/icons/lock";
import MapPinIcon from "lucide-solid/icons/map-pin";
import MessageSquareIcon from "lucide-solid/icons/message-square";
import MicIcon from "lucide-solid/icons/mic";
import MonitorIcon from "lucide-solid/icons/monitor";
import MonitorSmartphoneIcon from "lucide-solid/icons/monitor-smartphone";
import MoonIcon from "lucide-solid/icons/moon";
import MouseIcon from "lucide-solid/icons/mouse";
import NetworkIcon from "lucide-solid/icons/network";
import NotebookPenIcon from "lucide-solid/icons/notebook-pen";
import PaletteIcon from "lucide-solid/icons/palette";
import PanelsTopLeftIcon from "lucide-solid/icons/panels-top-left";
import PencilIcon from "lucide-solid/icons/pencil";
import PhoneIcon from "lucide-solid/icons/phone";
import PowerIcon from "lucide-solid/icons/power";
import PrinterIcon from "lucide-solid/icons/printer";
import PuzzleIcon from "lucide-solid/icons/puzzle";
import RefreshCwIcon from "lucide-solid/icons/refresh-cw";
import RotateCcwClockIcon from "lucide-solid/icons/rotate-ccw-clock";
import ScanIcon from "lucide-solid/icons/scan";
import ScreenShareIcon from "lucide-solid/icons/screen-share";
import SearchIcon from "lucide-solid/icons/search";
import SendIcon from "lucide-solid/icons/send";
import SettingsIcon from "lucide-solid/icons/settings";
import ShieldIcon from "lucide-solid/icons/shield";
import ShieldCheckIcon from "lucide-solid/icons/shield-check";
import SlidersHorizontalIcon from "lucide-solid/icons/sliders-horizontal";
import TagsIcon from "lucide-solid/icons/tags";
import TerminalIcon from "lucide-solid/icons/terminal";
import TimerIcon from "lucide-solid/icons/timer";
import TrashIcon from "lucide-solid/icons/trash";
import TriangleExclamationPointIcon from "lucide-solid/icons/triangle-alert";
import UnplugIcon from "lucide-solid/icons/unplug";
import UserIcon from "lucide-solid/icons/user";
import Volume2Icon from "lucide-solid/icons/volume-2";
import WebhookIcon from "lucide-solid/icons/webhook";
import WifiIcon from "lucide-solid/icons/wifi";
import WorkflowIcon from "lucide-solid/icons/workflow";
import XIcon from "lucide-solid/icons/x";
import ZapIcon from "lucide-solid/icons/zap";

const ICONS = {
  devices: MonitorSmartphoneIcon,
  activity: ActivityIcon,
  transfers: ArrowUpDownIcon,
  notes: NotebookPenIcon,
  timers: TimerIcon,
  readLater: BookmarkIcon,
  search: SearchIcon,
  settings: SettingsIcon,
  help: CircleQuestionMarkIcon,
  alert: TriangleExclamationPointIcon,
  "send-file": FileUpIcon,
  "send-folder": FolderUpIcon,
  "send-clip": ClipboardCopyIcon,
  "get-clip": ClipboardPasteIcon,
  ring: BellRingIcon,
  more: EllipsisIcon,
  connect: LinkIcon,
  edit: PencilIcon,
  layout: PanelsTopLeftIcon,
  verify: ShieldCheckIcon,
  reconnect: RefreshCwIcon,
  disconnect: UnplugIcon,
  remove: TrashIcon,
  map: NetworkIcon,
  refresh: RefreshCwIcon,
  close: XIcon,
  check: CheckIcon,
  chat: MessageSquareIcon,
  overview: LayoutDashboardIcon,
  files: FolderIcon,
  "folder-open": FolderOpenIcon,
  "folder-sync": FolderSyncIcon,
  notifications: BellIcon,
  media: CirclePlayIcon,
  control: SlidersHorizontalIcon,
  history: RotateCcwClockIcon,
  info: InfoIcon,
  terminal: TerminalIcon,
  screen: ScreenShareIcon,
  camera: CameraIcon,
  scan: ScanIcon,
  music: AudioLinesIcon,
  mic: MicIcon,
  speaker: Volume2Icon,
  image: ImageIcon,
  locate: MapPinIcon,
  shield: ShieldIcon,
  send: SendIcon,
  apps: LayoutGridIcon,
  clipboard: ClipboardIcon,
  calendar: CalendarClockIcon,
  workflow: WorkflowIcon,
  webhook: WebhookIcon,
  palette: PaletteIcon,
  languages: LanguagesIcon,
  code: CodeIcon,
  puzzle: PuzzleIcon,
  archive: ArchiveIcon,
  download: DownloadIcon,
  printer: PrinterIcon,
  phone: PhoneIcon,
  power: PowerIcon,
  mouse: MouseIcon,
  keyboard: KeyboardIcon,
  lock: LockIcon,
  key: KeyIcon,
  user: UserIcon,
  monitor: MonitorIcon,
  "app-window": AppWindowIcon,
  moon: MoonIcon,
  battery: BatteryMediumIcon,
  tags: TagsIcon,
  health: HeartPulseIcon,
  wifi: WifiIcon,
  bluetooth: BluetoothIcon,
  clock: ClockIcon,
  "link-direct": ZapIcon,
  "link-relay": GlobeIcon,
  "link-connecting": LoaderIcon,
  "link-waking": PowerIcon,
  "link-asleep": MoonIcon,
  "link-failed": TriangleExclamationPointIcon,
  "link-offline": CircleOffIcon,
  "chevron-down": ChevronDownIcon,
  "chevron-right": ChevronRightIcon,
  "ok": CircleCheckIcon,
  "no": CircleXIcon,
  "cpu": CpuIcon,
  "storage": HardDriveIcon,
  "memory": MemoryStickIcon,
  "tools": WrenchIcon,
  "remote": MousePointer2Icon,
  "call": PhoneCallIcon,
  "security": ShieldHalfIcon,
  "arrow-right": ArrowRightIcon,
  "quick": ZapIcon,
  "bell-dot": BellDotIcon,
  "find": MapPinIcon,
  "home": LayoutDashboardIcon,
} satisfies Record<string, Component<LucideProps>>;

export type IconName = keyof typeof ICONS;

/** A category's colour (tokens --c-*): files blue, messages violet, calls
 *  green, alerts amber, danger red, devices cyan. */
export type Tone = "blue" | "violet" | "green" | "amber" | "red" | "cyan";

/** An icon in a round, softly tinted chip (dashboard rows, cards). */
export function IconChip(props: { name: IconName; tone?: Tone; size?: number; class?: string }) {
  return (
    <span class={`icon-chip ${props.tone ? `chip-${props.tone}` : ""} ${props.class ?? ""}`} aria-hidden="true">
      <Icon name={props.name} size={props.size ?? 18} />
    </span>
  );
}

/** 18 px in nav, tabs and buttons (default), 20 px on setting rows, 40–48 px
 *  in empty states. */
export function Icon(props: { name: IconName; size?: number; class?: string; tone?: Tone }) {
  const Glyph = () => {
    const C = ICONS[props.name];
    const tone = props.tone ? ` tone-${props.tone}` : "";
    return <C size={props.size ?? 18} strokeWidth={1.8} class={`icon${tone} ${props.class ?? ""}`} aria-hidden="true" />;
  };
  return <>{Glyph()}</>;
}
