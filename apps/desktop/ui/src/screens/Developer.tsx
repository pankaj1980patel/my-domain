// Hidden Developer screen: everything technical lives here, out of the way.

import { createResource, createSignal, For, Show } from "solid-js";
import { commands } from "../bindings";
import { refreshDevices, state, unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "../components/ui";

export default function Developer(props: { onBack: () => void }) {
  const [stats, { refetch }] = createResource(() => commands.linkStats());
  const [name, setName] = createSignal("");
  const [ip, setIp] = createSignal("");
  const [port, setPort] = createSignal("");
  const [filter, setFilter] = createSignal("");

  const events = () => state.devLog.filter((e) => !filter() || e.name.includes(filter())).slice(0, 200);

  return (
    <section class="page">
      <button class="link back" onClick={props.onBack}>
        ← Settings
      </button>
      <header class="page-head">
        <h1>Developer</h1>
      </header>

      <article class="tile">
        <h2>This device</h2>
        <dl class="kv">
          <dt>Node</dt>
          <dd>{state.me?.node_id}</dd>
          <dt>LAN IP</dt>
          <dd>{state.me?.ip}</dd>
          <dt>QUIC port</dt>
          <dd>{state.me?.quic_port}</dd>
          <dt>IPv6</dt>
          <dd>{state.me?.ipv6 ?? "none"}</dd>
          <dt>Router mapping</dt>
          <dd>{state.me?.mapped_quic ?? "none"}</dd>
          <dt>Server</dt>
          <dd>{state.account.server}</dd>
        </dl>
        <div class="row">
          <button
            class="btn btn-ghost btn-sm"
            onClick={async () => {
              try {
                const fs = await unwrap(commands.firewallCheck());
                toast(fs.inbound_blocked ? "Inbound connections are blocked" : "Inbound connections are allowed", fs.inbound_blocked ? "error" : "ok");
              } catch (e) {
                toast(friendlyError(e), "error");
              }
            }}
          >
            Firewall check
          </button>
          <button
            class="btn btn-ghost btn-sm"
            onClick={async () => {
              try {
                await unwrap(commands.fcmSelftest());
                toast("FCM self-test sent; the pong shows in the event log", "info");
              } catch (e) {
                toast(friendlyError(e), "error");
              }
            }}
          >
            FCM self-test
          </button>
          <button class="btn btn-ghost btn-sm" onClick={() => refreshDevices().catch((e) => toast(friendlyError(e), "error"))}>
            Refresh from server
          </button>
          <button class="btn btn-ghost btn-sm" onClick={() => commands.scanLan()}>
            Scan LAN
          </button>
        </div>
      </article>

      <article class="tile">
        <h2>Links</h2>
        <button class="btn btn-ghost btn-sm" onClick={refetch}>
          Refresh
        </button>
        <table class="table">
          <thead>
            <tr>
              <th>Device</th>
              <th>Transport</th>
              <th>RTT</th>
              <th>Loss</th>
              <th>Quality</th>
            </tr>
          </thead>
          <tbody>
            <For each={stats() ?? []} fallback={<tr><td colSpan={5} class="muted">No live links</td></tr>}>
              {(l) => (
                <tr>
                  <td>{state.peers.find((p) => p.node_id === l.node_id)?.name ?? l.node_id}</td>
                  <td>{l.transport}</td>
                  <td>{l.rtt_ms != null ? `${l.rtt_ms} ms` : "–"}</td>
                  <td>{l.loss_pct != null ? `${l.loss_pct.toFixed(1)} %` : "–"}</td>
                  <td>{l.label}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </article>

      <article class="tile">
        <h2>Peers</h2>
        <table class="table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Node</th>
              <th>Address</th>
              <th>Source</th>
              <th>NAT</th>
            </tr>
          </thead>
          <tbody>
            <For each={state.peers}>
              {(p) => (
                <tr>
                  <td>{p.name}</td>
                  <td class="mono">{p.node_id.slice(0, 12)}</td>
                  <td class="mono">
                    {p.ip}:{p.quic_port}
                    <Show when={p.mapped_quic}> · {p.mapped_quic}</Show>
                  </td>
                  <td>{p.source}</td>
                  <td>{p.nat}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </article>

      <article class="tile">
        <h2>Add a device by address</h2>
        <form
          class="row"
          onSubmit={async (e) => {
            e.preventDefault();
            try {
              await unwrap(commands.addManualPeer(name(), ip(), Number(port())));
              toast(`Added ${ip()}`, "ok");
            } catch (err) {
              toast(friendlyError(err), "error");
            }
          }}
        >
          <input placeholder="Name" aria-label="Name" value={name()} onInput={(e) => setName(e.currentTarget.value)} />
          <input placeholder="IP address" aria-label="IP address" value={ip()} onInput={(e) => setIp(e.currentTarget.value)} />
          <input
            placeholder="QUIC port"
            aria-label="QUIC port"
            value={port()}
            onInput={(e) => setPort(e.currentTarget.value)}
            inputmode="numeric"
          />
          <button class="btn btn-soft" type="submit" disabled={!ip() || !port()}>
            Add
          </button>
        </form>
      </article>

      <article class="tile">
        <h2>Event log</h2>
        <div class="row">
          <input
            placeholder="Filter by event name"
            aria-label="Filter by event name"
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
          />

          <button
            class="btn btn-ghost btn-sm"
            onClick={() => navigator.clipboard.writeText(JSON.stringify(state.devLog, null, 2)).then(() => toast("Copied", "ok"))}
          >
            Copy all
          </button>
        </div>
        <ul class="log">
          <For each={events()}>
            {(e) => (
              <li>
                <time>{new Date(e.ts).toLocaleTimeString()}</time> <strong>{e.name}</strong>{" "}
                <span class="mono">{JSON.stringify(e.payload).slice(0, 240)}</span>
              </li>
            )}
          </For>
        </ul>
      </article>
    </section>
  );
}
