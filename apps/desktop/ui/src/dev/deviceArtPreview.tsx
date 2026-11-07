// Screenshot mode only (?mock=1&preview=deviceArt, see mockEntry.ts): every
// DeviceArt case on one page, from the sample backend, until the device
// pages place it. The phone shares colours (a gradient), the MacBook a
// picture; the office PC, the lab laptop, this computer and the made-up
// tablet share nothing (their built-in wallpapers).

import { render } from "solid-js/web";
import { For } from "solid-js";
import "@fontsource-variable/inter";
import "../styles/tokens.css";
import "../styles/app.css";
import { initTheme } from "../lib/theme";
import DeviceArt, { DEFAULT_ARTS, DefaultArt, WallpaperShareSetting } from "../components/DeviceArt";
import { LAB, MACBOOK, OFFICE, PIXEL } from "./mockTauri";

initTheme();

const CASES: { label: string; node?: string; icon: string }[] = [
  { label: "Pixel 8 (colours)", node: PIXEL, icon: "android_phone" },
  { label: "MacBook (picture)", node: MACBOOK, icon: "mac_laptop" },
  { label: "Office PC (built-in)", node: OFFICE, icon: "win_desktop" },
  { label: "Lab laptop (built-in)", node: LAB, icon: "linux_laptop" },
  { label: "Tablet (built-in)", node: "tablet-sample", icon: "android_tablet" },
  { label: "This Mac mini (built-in)", icon: "mac_desktop" },
];

function Preview() {
  return (
    <main style={{ padding: "24px", display: "grid", gap: "28px" }}>
      <For each={["large", "medium", "small"] as const}>
        {(size) => (
          <section style={{ display: "flex", gap: "28px", "flex-wrap": "wrap", "align-items": "flex-end" }}>
            <For each={CASES}>
              {(c) => (
                <figure style={{ margin: 0, display: "grid", gap: "6px", "justify-items": "center" }}>
                  <DeviceArt node={c.node} icon={c.icon} size={size} title={c.label} />
                  <figcaption class="muted small">{c.label}</figcaption>
                </figure>
              )}
            </For>
          </section>
        )}
      </For>
      {/* Every built-in wallpaper, on a phone. */}
      <section style={{ display: "flex", gap: "20px" }}>
        <For each={DEFAULT_ARTS}>
          {(_, i) => <DeviceArt node={`art-${i()}`} icon="phone" size="medium" fallback={<DefaultArt index={i()} />} />}
        </For>
      </section>
      <section style={{ "max-width": "480px" }}>
        <WallpaperShareSetting />
      </section>
    </main>
  );
}

render(() => <Preview />, document.getElementById("root")!);
