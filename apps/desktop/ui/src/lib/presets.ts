// What a schedule or an automation can do: one action a device takes, with
// its arguments. Shared by the Schedules tile (another device), this
// computer's schedules in Settings, and Automations. Only the ones a device
// lists in its status (`actions`) are offered; `always` ones need nothing
// listed.

import type { Key } from "../i18n";

export type Preset = { id: string; action: string; args: Record<string, unknown>; label: Key; always?: boolean };

/** On another device (a phone, mostly). */
export const DEVICE_PRESETS: Preset[] = [
  { id: "ringer_normal", action: "ringer", args: { mode: "normal" }, label: "schedule.preset.ringerNormal" },
  { id: "ringer_vibrate", action: "ringer", args: { mode: "vibrate" }, label: "schedule.preset.ringerVibrate" },
  { id: "ringer_silent", action: "ringer", args: { mode: "silent" }, label: "schedule.preset.ringerSilent" },
  { id: "lock", action: "lock", args: {}, label: "schedule.preset.lock" },
  { id: "ring", action: "ring", args: {}, label: "schedule.preset.ring", always: true },
  { id: "torch_off", action: "flashlight", args: { on: false }, label: "schedule.preset.torchOff" },
  { id: "awake_off", action: "keep_awake", args: { minutes: 0 }, label: "schedule.preset.awakeOff" },
  { id: "media", action: "media", args: { key: "play_pause" }, label: "schedule.preset.media" },
];

/** On this computer. */
export const LOCAL_PRESETS: Preset[] = [
  { id: "lock", action: "lock", args: {}, label: "schedule.preset.lockComputer" },
  { id: "mute", action: "mute", args: { on: true }, label: "schedule.preset.mute" },
  { id: "unmute", action: "mute", args: { on: false }, label: "schedule.preset.unmute" },
  // Media keys only toggle, so it says so.
  { id: "media", action: "media", args: { key: "play_pause" }, label: "schedule.preset.media" },
  { id: "awake_off", action: "keep_awake", args: { minutes: 0 }, label: "schedule.preset.awakeOffComputer" },
];

/** Automations only: no action, just a notification here. */
export const NOTIFY_PRESET: Preset = { id: "notify", action: "notify", args: {}, label: "automation.preset.notify", always: true };

/** Another computer that said it may be woken (`canWake`): this computer
 *  sends the wake-up, so it works while that one sleeps. Offered only then,
 *  e.g. "every weekday at 08:55, wake the office PC". */
export const WAKE_PRESET: Preset = { id: "wake", action: "wake", args: {}, label: "schedule.preset.wake", always: true };

/** `presets`, with waking when the device can be woken. */
export const withWake = (presets: Preset[], canWake: boolean) => (canWake ? [...presets, WAKE_PRESET] : presets);

/** The presets `actions` allows (`always` ones regardless). */
export const offeredPresets = (presets: Preset[], actions: string[] | undefined) =>
  presets.filter((p) => p.always || actions?.includes(p.action));
