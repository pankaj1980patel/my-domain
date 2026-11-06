// "Check connection": how this computer reaches a device, in plain words
// (connected directly on the home network, through the relay server, or not
// reachable and why), the details behind it, and a speed test.

import { createSignal, For, onCleanup, Show } from "solid-js";
import { peerName, state } from "../../lib/core";
import { cancelSpeedTest, diag, diagnose, speedBytes, speedRunning, startSpeedTest, type Diagnosis } from "../../lib/diag";
import { friendlyError, humanSize, timeAgo } from "../../lib/words";
import { toast } from "../../components/ui";
import { t, type Key } from "../../i18n";

const VERDICT: Record<string, Key> = {
  lan: "conn.verdict.lan",
  direct: "conn.verdict.direct",
  punched: "conn.verdict.punched",
  webrtc: "conn.verdict.webrtc",
  relay: "conn.verdict.relay",
  ble: "conn.verdict.ble",
};

const REASON: Record<string, Key> = {
  push_failed: "conn.reason.pushFailed",
  no_answer: "conn.reason.noAnswer",
  relay_no_answer: "conn.reason.relayNoAnswer",
  relay_unreachable: "conn.reason.relayUnreachable",
  no_path: "conn.reason.noPath",
  dropped: "conn.reason.dropped",
  key_changed: "conn.reason.keyChanged",
};

const STAGE: Record<string, Key> = {
  quic: "conn.stage.quic",
  signaled_quic: "conn.stage.signaled_quic",
  relay: "conn.stage.relay",
  punching: "conn.stage.punching",
  webrtc: "conn.stage.webrtc",
  nat_skip: "conn.stage.nat_skip",
  connected: "conn.stage.connected",
  failed: "conn.stage.failed",
  push_failed: "conn.stage.push_failed",
  no_answer: "conn.stage.no_answer",
  relay_no_answer: "conn.stage.relay_no_answer",
  relay_unreachable: "conn.stage.relay_unreachable",
  link_up: "conn.stage.link_up",
  link_down: "conn.stage.link_down",
  key_changed: "conn.stage.key_changed",
  key_trusted: "conn.stage.key_trusted",
  waking: "conn.stage.waking",
  asleep: "conn.stage.asleep",
  reconnected: "conn.stage.reconnected",
  lan_try: "conn.stage.lan_try",
};

/** Why a link went (`features::reconnect`). */
const DROP: Record<string, Key> = {
  idle: "conn.drop.idle",
  peer_idle: "conn.drop.peer_idle",
  network_change: "conn.drop.network_change",
  timeout: "conn.drop.timeout",
  dead: "conn.drop.dead",
  manual: "conn.drop.manual",
  peer_quit: "conn.drop.peer_quit",
  closed: "conn.drop.closed",
  lost: "conn.drop.lost",
};

const REACH: Record<string, Key> = {
  waking: "conn.reach.waking",
  asleep: "conn.reach.asleep",
  unreachable: "conn.reach.unreachable",
};

/** A reconnect time: "850 ms", "2.4 s". */
export function msText(x: number): string {
  return x < 1000 ? t("conn.dur.ms", { n: Math.round(x) }) : t("conn.dur.s", { n: (x / 1000).toFixed(1) });
}

/** A link's lifetime: "40 s", "12 min", "3 h". */
export function secsText(s: number): string {
  if (s < 60) return t("conn.dur.s", { n: s });
  if (s < 3600) return t("conn.dur.min", { n: Math.floor(s / 60) });
  return t("conn.dur.h", { n: Math.floor(s / 3600) });
}

const TRANSPORT: Record<string, Key> = {
  quic: "conn.transport.quic",
  punch: "conn.transport.punch",
  webrtc: "conn.transport.webrtc",
  relay: "conn.transport.relay",
  ble: "conn.transport.ble",
};

const NAT: Record<string, Key> = {
  easy: "conn.nat.easy",
  symmetric: "conn.nat.symmetric",
  unknown: "conn.nat.unknown",
};

const ago = (s: number) => timeAgo(Date.now() - s * 1000);
const ms = (x: number | null | undefined) => (x == null ? "–" : x < 10 ? x.toFixed(1) : Math.round(x).toString());

/** The one line that says how it's connected, or why it isn't. */
export function verdictLine(d: Diagnosis, name: string): string {
  const v = VERDICT[d.verdict];
  if (v) return t(v);
  if (d.verdict === "unreachable" && d.last_try_ago_s != null) {
    const why = d.reason ? REASON[d.reason] : undefined;
    return why
      ? t("conn.verdict.unreachableWhy", { ago: ago(d.last_try_ago_s), why: t(why, { name }) })
      : t("conn.verdict.unreachable", { ago: ago(d.last_try_ago_s) });
  }
  return t("conn.verdict.notConnected", { name });
}

/** What would help, if anything. */
function tips(d: Diagnosis, name: string): string[] {
  const out: string[] = [];
  const reach = d.reconnect.reach ? REACH[d.reconnect.reach] : undefined;
  if (!d.link && reach) out.push(t(reach, { name }));
  if (!d.link && d.verdict === "unreachable") out.push(t("conn.tip.unreachable", { name }));
  if (d.both_symmetric) out.push(t("conn.tip.symmetric"));
  if (!d.relay.signed_in) out.push(t("conn.tip.signedOut"));
  else if (d.relay.reachable === false) out.push(t("conn.tip.relayDown"));
  if (d.link && !d.ping_supported) out.push(t("conn.tip.oldApp", { name }));
  return out;
}

function addr(a: Diagnosis["peer"]): string {
  const parts = [a.ip && a.quic_port ? `${a.ip}:${a.quic_port}` : a.ip, a.ipv6, a.mapped_quic, a.public];
  return parts.filter((x, i) => x && parts.indexOf(x) === i).join(", ") || "–";
}

function relayLine(d: Diagnosis): string {
  if (!d.relay.signed_in) return t("conn.relay.none");
  if (d.relay.connected) return t("conn.relay.connected");
  return d.relay.reachable ? t("conn.relay.ok") : t("conn.relay.down");
}

export default function ConnectionTile(props: { node: string }) {
  const [busy, setBusy] = createSignal(false);
  const name = () => peerName(props.node);
  const report = () => diag.reports[props.node];
  const run = () => diag.speed[props.node];
  const linked = () => !!state.links[props.node];
  const bytes = () => speedBytes(state.links[props.node]);
  // Leaving the page stops a test that's running.
  onCleanup(() => {
    if (speedRunning(props.node)) cancelSpeedTest(props.node).catch(() => {});
  });

  const check = async () => {
    setBusy(true);
    try {
      await diagnose(props.node);
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const speed = async () => {
    try {
      await startSpeedTest(props.node, bytes());
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  const phaseLine = () => {
    const r = run();
    if (!r) return "";
    if (r.phase === "ping") return t("conn.speedPhase.ping");
    if (r.phase === "upload") return t("conn.speedPhase.upload", { name: name() });
    return t("conn.speedPhase.download", { name: name() });
  };
  // Ping, then each way: a bar across all three.
  const pct = () => {
    const r = run();
    if (!r || !r.total) return 0;
    const part = r.done / r.total;
    if (r.phase === "ping") return 2;
    return Math.round(r.phase === "upload" ? 5 + part * 45 : 50 + part * 50);
  };

  return (
    <article class="tile tile-wide conn-tile">
      <div class="tile-head">
        <h2>{t("conn.title")}</h2>
        <Show when={report()}>
          <span class="muted small">{t("conn.checked", { ago: timeAgo(diag.at[props.node] ?? Date.now()) })}</span>
        </Show>
      </div>
      <Show when={!report()}>
        <p class="muted small">{t("conn.hint", { name: name() })}</p>
      </Show>
      <Show when={report()}>
        {(d) => (
          <div class="stack conn-report" aria-live="polite">
            <p class="conn-verdict" classList={{ "conn-bad": !d().link, "conn-slow": !!d().link && !d().link!.direct }}>
              {verdictLine(d(), name())}
            </p>
            <For each={tips(d(), name())}>{(tip) => <p class="muted small">{tip}</p>}</For>
            <dl class="kv">
              <Show when={d().link}>
                {(l) => (
                  <>
                    <dt>{t("conn.how")}</dt>
                    <dd>
                      {TRANSPORT[l().transport] ? t(TRANSPORT[l().transport]) : l().transport}
                      {l().remote ? ` · ${l().remote}` : ""}
                    </dd>
                  </>
                )}
              </Show>
              <Show when={d().rtt}>
                {(r) => (
                  <>
                    <dt>{t("conn.rtt")}</dt>
                    <dd>
                      {r().samples > 0
                        ? t("conn.rttValue", { avg: ms(r().avg_ms), min: ms(r().min_ms), max: ms(r().max_ms) })
                        : t("conn.rttNone")}
                      <Show when={r().lost > 0 && r().samples > 0}>
                        {" · "}
                        {t("conn.rttLost", { lost: r().lost, total: r().lost + r().samples })}
                      </Show>
                    </dd>
                  </>
                )}
              </Show>
              <Show when={d().link?.loss_pct != null}>
                <dt>{t("conn.loss")}</dt>
                <dd>{t("conn.lossValue", { pct: (d().link?.loss_pct ?? 0).toFixed(1) })}</dd>
              </Show>
              <dt>{t("conn.itsAddress", { name: name() })}</dt>
              <dd>{addr(d().peer)}</dd>
              <dt>{t("conn.myAddress")}</dt>
              <dd>{addr(d().this_device)}</dd>
              <dt>{t("conn.network")}</dt>
              <dd>
                {t("conn.networkValue", {
                  mine: t(NAT[d().this_device.nat] ?? "conn.nat.unknown"),
                  name: name(),
                  theirs: t(NAT[d().peer.nat] ?? "conn.nat.unknown"),
                })}
              </dd>
              <dt>{t("conn.relay")}</dt>
              <dd>{relayLine(d())}</dd>
              <dt>{t("conn.reconnect")}</dt>
              <dd>
                {d().reconnect.samples > 0 && d().reconnect.last_ms != null && d().reconnect.avg_ms != null
                  ? t("conn.reconnectValue", {
                      last: msText(d().reconnect.last_ms!),
                      avg: msText(d().reconnect.avg_ms!),
                      count: d().reconnect.samples,
                    })
                  : t("conn.reconnectNone")}
              </dd>
              <Show when={d().reconnect.up_for_s != null}>
                <dt>{t("conn.upFor")}</dt>
                <dd>{secsText(d().reconnect.up_for_s!)}</dd>
              </Show>
              <Show when={d().reconnect.lifetime_last_s != null && d().reconnect.lifetime_avg_s != null}>
                <dt>{t("conn.lifetime")}</dt>
                <dd>
                  {t("conn.lifetimeValue", {
                    last: secsText(d().reconnect.lifetime_last_s!),
                    avg: secsText(d().reconnect.lifetime_avg_s!),
                  })}
                </dd>
              </Show>
              <Show when={d().reconnect.last_drop && d().reconnect.last_drop_ago_s != null}>
                <dt>{t("conn.lastDrop")}</dt>
                <dd>
                  {t("conn.lastDropValue", {
                    why: DROP[d().reconnect.last_drop!] ? t(DROP[d().reconnect.last_drop!]) : d().reconnect.last_drop!,
                    ago: ago(d().reconnect.last_drop_ago_s!),
                  })}
                </dd>
              </Show>
            </dl>
            <Show when={d().attempts.length > 0}>
              <h3 class="conn-sub">{t("conn.attempts")}</h3>
              <ul class="conn-attempts">
                <For each={d().attempts.slice(-6).reverse()}>
                  {(a) => (
                    <li>
                      <span>
                        {STAGE[a.stage] ? t(STAGE[a.stage]) : a.stage}
                        {a.stage === "link_up" && a.detail && TRANSPORT[a.detail] ? ` (${t(TRANSPORT[a.detail])})` : ""}
                      </span>
                      <span class="muted">{ago(a.ago_s)}</span>
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </div>
        )}
      </Show>

      <Show when={speedRunning(props.node)}>
        <div class="stack conn-speed" aria-live="polite">
          <span class="small">{phaseLine()}</span>
          <div
            class="progress"
            role="progressbar"
            aria-label={t("a11y.speedProgress")}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct()}
          >
            <span style={{ width: `${pct()}%` }} />
          </div>
        </div>
      </Show>
      <Show when={!speedRunning(props.node) && run()?.phase === "done" && run()?.result}>
        {(r) => (
          <div class="conn-results">
            <div class="conn-result">
              <span class="muted small">{t("conn.speedUp", { name: name() })}</span>
              <strong>{t("conn.mbps", { n: r().upload_mbps.toFixed(1) })}</strong>
            </div>
            <div class="conn-result">
              <span class="muted small">{t("conn.speedDown", { name: name() })}</span>
              <strong>{t("conn.mbps", { n: r().download_mbps.toFixed(1) })}</strong>
            </div>
            <div class="conn-result">
              <span class="muted small">{t("conn.rtt")}</span>
              <strong>{r().rtt.samples > 0 ? `${ms(r().rtt.avg_ms)} ms` : "–"}</strong>
            </div>
          </div>
        )}
      </Show>
      <Show when={!speedRunning(props.node) && run()?.phase === "failed"}>
        <p class="small text-error">{t("conn.speedFailed", { why: run()?.error ?? "" })}</p>
      </Show>
      <Show when={!speedRunning(props.node) && run()?.phase === "cancelled"}>
        <p class="muted small">{t("conn.speedCancelled")}</p>
      </Show>

      <div class="row">
        <button class="btn btn-soft" disabled={busy()} onClick={check}>
          {busy() ? t("conn.checking") : t("conn.check")}
        </button>
        <Show
          when={speedRunning(props.node)}
          fallback={
            <button
              class="btn btn-ghost"
              disabled={!linked()}
              title={linked() ? t("conn.speedHint", { size: humanSize(bytes()) }) : t("conn.speedNeedsLink", { name: name() })}
              onClick={speed}
            >
              {t("conn.speed")}
            </button>
          }
        >
          <button class="btn btn-ghost" onClick={() => cancelSpeedTest(props.node).catch(() => {})}>
            {t("conn.speedCancel")}
          </button>
        </Show>
      </div>
    </article>
  );
}
