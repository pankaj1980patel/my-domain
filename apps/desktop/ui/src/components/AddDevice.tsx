// "Add a device" (Settings → Account): a one-time QR code of a join link,
// and the same invite as a 6-digit code, valid 5 minutes (mdcore `join`).
// Watches for the new device; when it joins, says so and closes. Closing it
// withdraws the invite.

import { createSignal, onCleanup, Show } from "solid-js";
import { commands, type JoinInvite } from "../bindings";
import { refreshDevices, unwrap } from "../lib/core";
import { friendlyError, niceName } from "../lib/words";
import { toast } from "./ui";
import { t } from "../i18n";
import { focusSoon } from "../lib/a11y";
import { OwnFingerprint } from "./VerifyDevice";

const POLL_MS = 2000;

/** "4:59" */
function mmss(secs: number): string {
  const s = Math.max(0, Math.ceil(secs));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function AddDevice() {
  const [open, setOpen] = createSignal(false);
  const [invite, setInvite] = createSignal<JoinInvite>();
  const [left, setLeft] = createSignal(0);
  const [error, setError] = createSignal("");
  let timer: number | undefined;
  let polling = false;
  let openButton: HTMLButtonElement | undefined;
  let closeButton: HTMLButtonElement | undefined;

  const stop = () => {
    if (timer !== undefined) clearInterval(timer);
    timer = undefined;
  };

  const tick = async () => {
    if (!invite()) return;
    setLeft((n) => n - 1);
    if (left() <= 0) {
      stop();
      return;
    }
    // Ask the server every couple of seconds (one request at a time).
    if (polling || left() % (POLL_MS / 1000) !== 0) return;
    polling = true;
    try {
      const s = await unwrap(commands.joinStatus());
      if (s.state === "joined") {
        toast(t("join.joined", { name: niceName(s.name ?? "") }), "ok");
        close(false);
        refreshDevices().catch(() => {});
      } else if (s.state === "none") {
        setLeft(0);
        stop();
      }
    } catch {
      // Offline for a moment: try again on the next tick.
    } finally {
      polling = false;
    }
  };

  const start = async () => {
    stop();
    setInvite(undefined);
    setError("");
    setOpen(true);
    focusSoon(() => closeButton);
    try {
      const inv = await unwrap(commands.joinInvite());
      if (!open()) {
        commands.joinCancel().catch(() => {});
        return;
      }
      setInvite(inv);
      setLeft(inv.expires_in);
      timer = window.setInterval(tick, 1000);
    } catch (e) {
      setError(friendlyError(e));
    }
  };

  const close = (withdraw = true) => {
    stop();
    if (withdraw && invite() && left() > 0) commands.joinCancel().catch(() => {});
    setOpen(false);
    setInvite(undefined);
    focusSoon(() => openButton);
  };

  onCleanup(() => {
    if (open()) close();
  });

  const side = () => (invite()?.qr_size ?? 21) + 8;
  const spaced = (code: string) => `${code.slice(0, 3)} ${code.slice(3)}`;

  return (
    <Show
      when={open()}
      fallback={
        <button class="btn btn-ghost" ref={openButton} onClick={start}>
          {t("join.addDevice")}
        </button>
      }
    >
      <div class="join-panel" role="group" aria-label={t("join.addDevice")}>
        <Show when={error()}>
          <p class="error" role="alert">
            {error()}
          </p>
        </Show>
        <Show when={!invite() && !error()}>
          <p class="muted">{t("join.making")}</p>
        </Show>
        <Show when={invite()}>
          {(inv) => (
            <Show
              when={left() > 0}
              fallback={
                <div class="join-expired">
                  <p>{t("join.expired")}</p>
                  <button class="btn btn-primary btn-sm" onClick={start}>
                    {t("join.newCode")}
                  </button>
                </div>
              }
            >
              <svg
                class="join-qr"
                viewBox={`0 0 ${side()} ${side()}`}
                role="img"
                aria-label={t("join.qrLabel")}
                shape-rendering="crispEdges"
              >
                <rect width={side()} height={side()} fill="#fff" />
                <path d={inv().qr_path} fill="#000" />
              </svg>
              <p class="muted small">{t("join.scan")}</p>
              <p class="join-code">
                {spaced(inv().code)}
              </p>
              <p class="muted small">
                {t("join.expiresIn", { time: mmss(left()) })}
              </p>
            </Show>
          )}
        </Show>
        <p class="muted small">{t("join.privacy")}</p>
        <OwnFingerprint />
        <button class="btn btn-ghost btn-sm" ref={closeButton} onClick={() => close()}>
          {t("join.close")}
        </button>
      </div>
    </Show>
  );
}
