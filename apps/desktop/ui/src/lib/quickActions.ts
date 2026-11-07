// One-click actions for a device, shared by the home cards and the device
// page's header. Each is a function the app already had (the command
// palette and the device page's tiles use the same ones), with its toast.

import { peerName, pickAndSendFile, pickAndSendFolder, pullClipboard, ringDevice, sendClipboardTo, state } from "./core";
import { friendlyError } from "./words";
import { toast } from "../components/ui";
import { t, tn } from "../i18n";

const fail = (e: unknown) => toast(friendlyError(e), "error");

/** Choose a file and send it (it waits for a device that isn't reachable). */
export function quickSendFile(node: string) {
  const name = peerName(node);
  return pickAndSendFile(node).then((sent) => sent && toast(tn("files.started", 1, { name }), "ok"), fail);
}

/** Choose a folder and send everything in it. */
export function quickSendFolder(node: string) {
  const name = peerName(node);
  return pickAndSendFolder(node).then((n) => n > 0 && toast(tn("files.started", n, { name }), "ok"), fail);
}

/** What's copied here, to the device's clipboard (queued when unreachable). */
export function quickSendClipboard(node: string) {
  const name = peerName(node);
  return sendClipboardTo(node).then(
    // "cancelled": it looked like a password and you said not to send it.
    (via) =>
      via !== "cancelled" &&
      toast(via === "queued" ? t("palette.clipQueued", { name }) : t("palette.clipSent", { name }), "ok"),
    fail,
  );
}

/** The device's clipboard, copied here. Only over a live link. */
export function quickGetClipboard(node: string) {
  return pullClipboard(node).then(() => toast(t("clipboard.copied"), "ok"), fail);
}

/** Ring it (find my device); queued when unreachable. */
export function quickRing(node: string) {
  const name = peerName(node);
  return ringDevice(node).then(
    (via) => toast(via === "queued" ? t("control.ringQueued", { name }) : t("control.ringing", { name }), "ok"),
    fail,
  );
}

/** Why "Get clipboard" can't run now (not connected), else undefined. */
export function needsLink(node: string): string | undefined {
  return state.links[node] ? undefined : t("palette.offline", { name: peerName(node) });
}
