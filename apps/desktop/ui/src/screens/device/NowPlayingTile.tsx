// Now playing and media controls (backend `media_ctl.rs`, mdcore
// `features::media`): what plays on another of your devices (artwork,
// title, artist, app, where it is) with previous / play-pause / next, a
// seek bar and its volume, and which app to show when several play. While
// the tile is open it asks the device to keep it posted (`media_watch`
// every few minutes); the position runs on here between updates. A device
// whose app is older shows what plays, without controls.

import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { clock, mediaCommand, peerName, positionAt, state, watchMedia } from "../../lib/core";
import { t } from "../../i18n";
import { HelpLink } from "../../help/HelpLink";

/** How often to ask to stay posted (the device keeps a watcher 10 min). */
const WATCH_EVERY_MS = 4 * 60_000;

export default function NowPlayingTile(props: { node: string }) {
  const name = () => peerName(props.node);
  const p = () => state.playing[props.node];
  const controls = () => !!state.status[props.node]?.mediaCtl;
  const [tick, setTick] = createSignal(Date.now());
  // While dragging the seek bar: where it is (the position doesn't jump back).
  const [dragging, setDragging] = createSignal<number | null>(null);

  onMount(() => {
    if (controls()) watchMedia(props.node);
    const watch = setInterval(() => controls() && watchMedia(props.node), WATCH_EVERY_MS);
    const clockTick = setInterval(() => setTick(Date.now()), 1000);
    onCleanup(() => {
      clearInterval(watch);
      clearInterval(clockTick);
    });
  });

  const pos = createMemo(() => {
    const cur = p();
    return cur ? positionAt(cur, tick()) : undefined;
  });
  const shownPos = () => dragging() ?? pos() ?? 0;
  const cmd = (action: string, value?: number, session?: string) =>
    void mediaCommand(props.node, action, value, session);

  return (
    <article class="tile now-playing">
      <h2>
        {t("media.title")}
        <HelpLink topic="audio" />
      </h2>
      <Show when={p()} fallback={<p class="muted small">{t("media.nothing", { device: name() })}</p>}>
        {(cur) => (
          <>
            <div class="row np-head">
              <Show
                when={cur().art}
                fallback={
                  <div class="np-art np-art-empty" aria-hidden="true">
                    ♪
                  </div>
                }
              >
                <img class="np-art" src={cur().art} alt={t("media.art", { title: cur().title })} />
              </Show>
              <div class="np-text">
                <strong class="np-title">{cur().title}</strong>
                <Show when={cur().artist}>
                  <span class="small">{cur().artist}</span>
                </Show>
                <span class="muted small row np-app">
                  <Show when={cur().icon}>
                    <img class="np-icon" src={cur().icon} alt="" />
                  </Show>
                  {cur().playing
                    ? t("media.playingIn", { app: cur().app ?? name() })
                    : t("media.pausedIn", { app: cur().app ?? name() })}
                </span>
              </div>
            </div>

            <Show when={controls() && cur().duration}>
              <div class="np-seek">
                <input
                  type="range"
                  min="0"
                  max={cur().duration}
                  step="1000"
                  value={shownPos()}
                  disabled={cur().canSeek === false}
                  aria-label={t("media.seek")}
                  aria-valuetext={clock(shownPos())}
                  onInput={(e) => setDragging(Number(e.currentTarget.value))}
                  onChange={(e) => {
                    const to = Number(e.currentTarget.value);
                    setDragging(null);
                    cmd("seek", to, cur().session);
                  }}
                />
                <div class="row small muted np-times">
                  <span>{clock(shownPos())}</span>
                  <span>{clock(cur().duration ?? 0)}</span>
                </div>
              </div>
            </Show>

            <Show
              when={controls()}
              fallback={<p class="muted small">{t("media.olderApp", { device: name() })}</p>}
            >
              <div class="row np-buttons" role="group" aria-label={t("media.controls")}>
                <button
                  class="btn btn-soft btn-sm"
                  disabled={cur().canPrev === false}
                  aria-label={t("media.previous")}
                  title={t("media.previous")}
                  onClick={() => cmd("previous", undefined, cur().session)}
                >
                  ⏮
                </button>
                <button
                  class="btn btn-primary btn-sm"
                  aria-label={cur().playing ? t("media.pause") : t("media.play")}
                  title={cur().playing ? t("media.pause") : t("media.play")}
                  onClick={() => cmd(cur().playing ? "pause" : "play", undefined, cur().session)}
                >
                  {cur().playing ? "⏸" : "▶"}
                </button>
                <button
                  class="btn btn-soft btn-sm"
                  disabled={cur().canNext === false}
                  aria-label={t("media.next")}
                  title={t("media.next")}
                  onClick={() => cmd("next", undefined, cur().session)}
                >
                  ⏭
                </button>
              </div>

              <div class="row small np-volume">
                <button
                  class="btn btn-ghost btn-sm"
                  aria-label={t("media.volumeDown")}
                  title={t("media.volumeDown")}
                  onClick={() => cmd("volume_down")}
                >
                  −
                </button>
                <Show
                  when={cur().volumeMax}
                  fallback={<span class="muted">{t("media.volume")}</span>}
                >
                  <input
                    type="range"
                    min="0"
                    max={cur().volumeMax}
                    step="1"
                    value={cur().volume ?? 0}
                    aria-label={t("media.volume")}
                    onChange={(e) => cmd("volume_set", Number(e.currentTarget.value))}
                  />
                </Show>
                <button
                  class="btn btn-ghost btn-sm"
                  aria-label={t("media.volumeUp")}
                  title={t("media.volumeUp")}
                  onClick={() => cmd("volume_up")}
                >
                  +
                </button>
              </div>

              <Show when={cur().sessions.length > 1}>
                <label class="row small np-sessions">
                  <span class="muted">{t("media.session")}</span>
                  <select onChange={(e) => cmd("session", undefined, e.currentTarget.value)}>
                    <For each={cur().sessions}>
                      {(s) => (
                        <option value={s.id} selected={s.id === cur().session}>
                          {(s.app ?? s.id) + (s.title ? " · " + s.title : "") + (s.playing ? " ▶" : "")}
                        </option>
                      )}
                    </For>
                  </select>
                </label>
              </Show>
            </Show>
          </>
        )}
      </Show>
    </article>
  );
}
