// What a device is, in words, for its page: its operating system's name.

import { state } from "./core";

/** Product names: not translated. */
const OS: Record<string, string> = {
  macos: "macOS",
  windows: "Windows",
  linux: "Linux",
  android: "Android",
  ios: "iOS",
};

/** The device's OS ("Android", "macOS"); "" when it hasn't said. */
export function osName(node: string): string {
  const os = state.kinds[node]?.os || state.status[node]?.platform || "";
  return OS[os] ?? "";
}
