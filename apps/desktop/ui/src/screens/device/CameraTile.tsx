// The phone's camera on this computer (backend `phone_screen.rs`, mdcore
// `features::camera`): look at something across the room, or use the phone
// as a document camera. "Ask to see its camera" asks the phone; its owner
// says yes there (every time), and the phone shows that its camera is on
// until it stops. While it shows: switch camera, torch, take a photo (a
// full-size picture that arrives in Downloads as a file), full screen,
// stop. The pictures are drawn on a canvas and never kept.

import { createSignal, onCleanup, Show } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import { commands } from "../../bindings";
import { peerName, setState } from "../../lib/core";
import { friendlyError } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";
import { NearbyBadge } from "../../components/NearbyBadge";
import { focusSoon, rememberFocus, trapTab } from "../../lib/a11y";
import { HelpLink } from "../../help/HelpLink";

type Offer = { from: string; on: boolean; front: boolean; torch: boolean | null; reason: string | null };

/** What the phone said when its camera went off, for a toast. */
function offReason(reason: string | null, name: string): string {
  switch (reason) {
    case "declined":
      return t("camera.declined", { name });
    case "locked":
      return t("camera.locked", { name });
    case "no_permission":
      return t("camera.noPermission", { name });
    case "busy":
      return t("camera.busy", { name });
    case "error":
      return t("camera.error", { name });
    default:
      return t("camera.stopped", { name });
  }
}

export default function CameraTile(props: { node: string }) {
  // off: not asked; asking: waiting for the phone's owner; on: it shows.
  const [phase, setPhase] = createSignal<"off" | "asking" | "on">("off");
  const [hasFrame, setHasFrame] = createSignal(false);
  const [full, setFull] = createSignal(false);
  const [front, setFront] = createSignal(false);
  const [torch, setTorch] = createSignal<boolean | null>(null);
  const name = () => peerName(props.node);
  let canvas: HTMLCanvasElement | undefined;
  let stage: HTMLDivElement | undefined;
  let exitFullButton: HTMLButtonElement | undefined;
  let toggleButton: HTMLButtonElement | undefined;
  let restoreFull: (() => void) | undefined;
  let renew: ReturnType<typeof setInterval> | undefined;
  let unlisten: (() => void)[] = [];

  const enterFull = () => {
    restoreFull = rememberFocus();
    setFull(true);
    focusSoon(() => exitFullButton);
  };
  const exitFull = () => {
    setFull(false);
    restoreFull?.();
    restoreFull = undefined;
  };

  // Draw each picture as it comes; a newer one replaces it before it's
  // drawn if pictures arrive faster than they decode.
  let latest = 0;
  const draw = (jpeg: string) => {
    const n = ++latest;
    const img = new Image();
    img.onload = () => {
      if (n !== latest || !canvas) return;
      if (canvas.width !== img.naturalWidth || canvas.height !== img.naturalHeight) {
        canvas.width = img.naturalWidth;
        canvas.height = img.naturalHeight;
      }
      canvas.getContext("2d")?.drawImage(img, 0, 0);
      setHasFrame(true);
    };
    img.src = `data:image/jpeg;base64,${jpeg}`;
  };

  const watch = async (on: boolean) => {
    const r = await commands.phoneCameraWatch(props.node, on);
    if (r.status === "error") throw new Error(r.error);
  };
  const stop = (tell = true) => {
    if (renew) clearInterval(renew);
    renew = undefined;
    unlisten.forEach((f) => f());
    unlisten = [];
    if (tell && phase() !== "off") watch(false).catch(() => {});
    const wasFull = full();
    setPhase("off");
    setHasFrame(false);
    setFull(false);
    setTorch(null);
    restoreFull = undefined;
    if (wasFull) focusSoon(() => toggleButton);
    latest++;
  };
  const start = async () => {
    if (phase() !== "off") return;
    setPhase("asking");
    try {
      unlisten.push(
        await listen<{ from: string; jpeg: string }>("camera-frame", (e) => {
          if (e.payload.from !== props.node || phase() === "off") return;
          setPhase("on");
          draw(e.payload.jpeg);
        }),
        // The phone's answer, its state (camera, torch), or it went off.
        await listen<Offer>("camera-offer", (e) => {
          const o = e.payload;
          if (o.from !== props.node || phase() === "off") return;
          if (o.on) {
            setPhase("on");
            setFront(o.front);
            setTorch(o.torch);
          } else {
            stop(false);
            toast(offReason(o.reason, name()), "info");
          }
        }),
      );
      await watch(true);
      renew = setInterval(() => watch(true).catch(() => {}), 10_000);
    } catch (err) {
      stop(false);
      toast(friendlyError(err), "error");
    }
  };

  const control = (op: "switch" | "torch" | "photo", on = false) =>
    commands.phoneCameraControl(props.node, op, on).then((r) => {
      if (r.status === "error") toast(friendlyError(r.error), "error");
      return r.status === "ok";
    });
  const takePhoto = async () => {
    setState("cameraPhotos", props.node, Date.now());
    if (await control("photo")) toast(t("camera.taking", { name: name() }), "info");
  };

  const leaveFull = (e: KeyboardEvent) => {
    if (e.key === "Escape" && full()) exitFull();
  };
  window.addEventListener("keydown", leaveFull);
  onCleanup(() => {
    window.removeEventListener("keydown", leaveFull);
    stop();
  });

  const buttons = () => (
    <>
      <button class="btn btn-ghost btn-sm" onClick={() => control("switch")} disabled={phase() !== "on"}>
        {front() ? t("camera.useBack") : t("camera.useFront")}
      </button>
      <Show when={torch() !== null}>
        <button
          class="btn btn-ghost btn-sm"
          aria-pressed={torch() === true}
          onClick={() => control("torch", !torch())}
          disabled={phase() !== "on"}
        >
          {torch() ? t("camera.torchOff") : t("camera.torchOn")}
        </button>
      </Show>
      <button class="btn btn-soft btn-sm" onClick={takePhoto} disabled={phase() !== "on"}>
        {t("camera.photo")}
      </button>
    </>
  );

  return (
    <article class="tile">
      <h2>
        {t("camera.title")} <HelpLink topic="screen" />
      </h2>
      <p class="muted small">{t("camera.hint")}</p>
      <Show when={phase() !== "off"}>
        <NearbyBadge node={props.node} />
        <div
          class="screen-stage camera-stage"
          classList={{ "screen-stage-full": full() }}
          ref={stage}
          role={full() ? "dialog" : undefined}
          aria-modal={full() ? "true" : undefined}
          aria-label={full() ? t("camera.title") : undefined}
          onKeyDown={(e) => full() && trapTab(e, stage)}
        >
          <Show when={full()}>
            <div class="screen-stage-bar">
              <strong>{t("camera.title")}</strong>
              <span class="grow" />
              {buttons()}
              <button class="btn btn-ghost btn-sm" ref={exitFullButton} onClick={exitFull}>
                {t("phoneScreen.exitFull")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => stop()}>
                {t("screen.stop")}
              </button>
            </div>
          </Show>
          <Show when={!hasFrame()}>
            <p class="muted small" role="status">
              {phase() === "asking" ? t("camera.waiting", { name: name() }) : t("camera.starting")}
            </p>
          </Show>
          <canvas
            ref={canvas}
            class="screen-view"
            classList={{ "screen-hidden": !hasFrame() }}
            role="img"
            aria-label={t("a11y.cameraView", { name: name() })}
          />
        </div>
        <Show when={!full()}>
          <div class="row camera-controls">{buttons()}</div>
        </Show>
      </Show>
      <div class="row">
        <Show when={phase() !== "off"}>
          <button class="btn btn-ghost btn-sm" onClick={enterFull} disabled={!hasFrame()}>
            {t("phoneScreen.full")}
          </button>
        </Show>
        <button
          ref={toggleButton}
          class={phase() !== "off" ? "btn btn-ghost btn-sm" : "btn btn-soft btn-block"}
          onClick={() => (phase() !== "off" ? stop() : start())}
        >
          {phase() !== "off" ? t("screen.stop") : t("camera.ask")}
        </button>
      </div>
    </article>
  );
}
