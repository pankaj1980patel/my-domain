// Device icons: the shape of the device (phone, laptop, iMac, tower...)
// with a small OS mark inside it (an apple for a Mac, four panes for
// Windows, a penguin for Linux, a robot head for Android). The marks are
// simple original glyphs, not the vendors' logos. One stroke style (24-unit
// grid, 1.8 stroke, round caps) in the current text colour, so they read
// at 16-48 px in both themes. The names are the core's
// (`features::device_kind::ICONS`).

import type { JSX } from "solid-js";

/** Every icon drawn here (same set and order as the core's `ICONS`). */
export const DEVICE_ICONS = [
  "android_phone",
  "android_tablet",
  "android_foldable",
  "phone",
  "tablet",
  "mac_laptop",
  "imac",
  "mac_desktop",
  "win_laptop",
  "win_desktop",
  "linux_laptop",
  "linux_desktop",
  "laptop",
  "desktop",
  "tv",
  "watch",
  "unknown",
] as const;

type Mark = "apple" | "windows" | "tux" | "android";

/** An OS mark about 5 units across, centred on (x, y). */
function OsMark(props: { mark: Mark; x: number; y: number; scale?: number }): JSX.Element {
  const s = () => props.scale ?? 1;
  return (
    <g transform={`translate(${props.x} ${props.y}) scale(${s()})`} stroke-width="0.9">
      {(() => {
        switch (props.mark) {
          case "apple":
            return (
              <>
                <path
                  d="M0 -1.3C-0.8 -1.9 -2.1 -1.7 -2.1 -0.1C-2.1 1.2 -1.1 2.5 -0.6 2.5C-0.3 2.5 -0.2 2.3 0 2.3C0.2 2.3 0.3 2.5 0.6 2.5C1.1 2.5 2.1 1.2 2.1 -0.1C2.1 -1.7 0.8 -1.9 0 -1.3Z"
                  fill="currentColor"
                  stroke="none"
                />
                <path d="M0 -1.5C0 -2.3 0.5 -2.8 1.1 -2.9" fill="none" />
              </>
            );
          case "windows":
            return (
              <g fill="currentColor" stroke="none">
                <rect x="-2" y="-2" width="1.75" height="1.75" rx="0.2" />
                <rect x="0.25" y="-2" width="1.75" height="1.75" rx="0.2" />
                <rect x="-2" y="0.25" width="1.75" height="1.75" rx="0.2" />
                <rect x="0.25" y="0.25" width="1.75" height="1.75" rx="0.2" />
              </g>
            );
          case "tux":
            return (
              <>
                <circle cx="0" cy="-1.6" r="1.1" fill="currentColor" stroke="none" />
                {/* The belly is a hole (wound the other way). */}
                <path d="M-1.8 2.3C-1.8 0.4 -1.1 -0.8 0 -0.8S1.8 0.4 1.8 2.3ZM0 0C-0.7 0 -1 0.9 -0.95 2H0.95C1 0.9 0.7 0 0 0Z" fill="currentColor" stroke="none" />
                <path d="M-1.6 0.2L-2.3 1.3M1.6 0.2L2.3 1.3M-1.5 2.6H-0.4M0.4 2.6H1.5" fill="none" />
              </>
            );
          case "android":
            return (
              <>
                {/* The eyes are holes (wound the other way). */}
                <path d="M-2.5 1.7A2.5 2.5 0 0 1 2.5 1.7ZM-1.25 0.75A0.35 0.35 0 1 0 -0.55 0.75A0.35 0.35 0 1 0 -1.25 0.75ZM0.55 0.75A0.35 0.35 0 1 0 1.25 0.75A0.35 0.35 0 1 0 0.55 0.75Z" fill="currentColor" stroke="none" />
                <path d="M-1.3 -0.5L-1.9 -1.5M1.3 -0.5L1.9 -1.5" fill="none" />
              </>
            );
        }
      })()}
    </g>
  );
}

const laptop = (mac: boolean) => (
  <>
    <rect x="4" y="5" width="16" height="11" rx={mac ? 2 : 1.2} />
    {mac ? <path d="M2 18.5h20M10.5 18.5v-.4h3v.4" /> : <path d="M2 19h20" />}
  </>
);

const monitor = (
  <>
    <rect x="3" y="4" width="18" height="12" rx="1.5" />
    <path d="M8 20h8M12 16v4" />
  </>
);

/** One device icon, drawn in the current text colour. Unknown names draw
 *  the "unknown" icon. */
export function DeviceIcon(props: { icon: string; size?: number; title?: string }) {
  const size = () => props.size ?? 24;
  return (
    <svg
      class="device-icon"
      width={size()}
      height={size()}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.8"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden={props.title ? undefined : "true"}
      role={props.title ? "img" : undefined}
      aria-label={props.title}
    >
      {(() => {
        switch (props.icon) {
          case "android_phone":
            return (
              <>
                <rect x="6.5" y="2" width="11" height="20" rx="2.2" />
                <path d="M11 18.5h2" />
                <OsMark mark="android" x={12} y={10.5} />
              </>
            );
          case "phone":
            return (
              <>
                <rect x="7" y="2" width="10" height="20" rx="2" />
                <path d="M11 18h2" />
              </>
            );
          case "android_tablet":
            return (
              <>
                <rect x="3.5" y="3" width="17" height="18" rx="2" />
                <path d="M11 18h2" />
                <OsMark mark="android" x={12} y={11} />
              </>
            );
          case "tablet":
            return (
              <>
                <rect x="4" y="2" width="16" height="20" rx="2" />
                <path d="M11 18h2" />
              </>
            );
          case "android_foldable":
            return (
              <>
                <rect x="2.5" y="4" width="19" height="16" rx="2" />
                <path d="M12 4v16" stroke-dasharray="1.6 1.6" />
                <OsMark mark="android" x={17} y={12} scale={0.8} />
              </>
            );
          case "mac_laptop":
            return (
              <>
                {laptop(true)}
                <OsMark mark="apple" x={12} y={10.6} />
              </>
            );
          case "win_laptop":
            return (
              <>
                {laptop(false)}
                <OsMark mark="windows" x={12} y={10.5} />
              </>
            );
          case "linux_laptop":
            return (
              <>
                {laptop(false)}
                <OsMark mark="tux" x={12} y={10.5} />
              </>
            );
          case "laptop":
            return laptop(false);
          case "imac":
            return (
              <>
                <rect x="2.5" y="3" width="19" height="13.5" rx="1.8" />
                <path d="M2.5 13.2h19M10.2 16.5l-.5 3.5h4.6l-.5-3.5M8.5 20h7" />
                <OsMark mark="apple" x={12} y={8.2} />
              </>
            );
          case "mac_desktop":
            return (
              <>
                <rect x="3" y="8" width="18" height="8" rx="2.4" />
                <path d="M6 18.5h12" />
                <OsMark mark="apple" x={12} y={12} scale={0.85} />
              </>
            );
          case "win_desktop":
            return (
              <>
                {monitor}
                <OsMark mark="windows" x={12} y={10} />
              </>
            );
          case "linux_desktop":
            return (
              <>
                <rect x="6.5" y="2" width="11" height="20" rx="1.5" />
                <path d="M9.5 5.5h5M9.5 8h5" />
                <OsMark mark="tux" x={12} y={14.5} />
              </>
            );
          case "desktop":
            return monitor;
          case "tv":
            return (
              <>
                <rect x="2" y="7" width="20" height="13" rx="2" />
                <path d="M8 3l4 4 4-4" />
              </>
            );
          case "watch":
            return (
              <>
                <rect x="6" y="6" width="12" height="12" rx="3" />
                <path d="M9 6l1-4h4l1 4M9 18l1 4h4l1-4" />
              </>
            );
          default:
            return (
              <>
                <rect x="4" y="4" width="16" height="16" rx="3" />
                <path d="M9.8 9.8a2.3 2.3 0 1 1 3.2 2.1c-.6.3-1 .8-1 1.4v.3" />
                <circle cx="12" cy="16.3" r="0.4" fill="currentColor" />
              </>
            );
        }
      })()}
    </svg>
  );
}
