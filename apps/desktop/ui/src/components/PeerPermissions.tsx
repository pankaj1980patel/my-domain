// Per-device permissions (backend peer_perms.rs, mdcore features::perms):
// which device may control this computer, see its screen, or take over its
// mouse and keyboard. All off until allowed for that device.
//
//  - PermissionsTile: a device page's "Permissions" switches.
//  - PermissionsSummary: Settings' summary (who is allowed what).
//  - PermissionAsk: "Pixel wants to control this computer — Allow once /
//    Always / Don't allow" when a device asks for what it isn't allowed
//    (`permission-ask`; the window comes up for it). Escape is no answer.
//    Also says why when another device refused this one
//    (`permission-refused`).
//  - PermissionNotice: once, after the upgrade from the global switches:
//    what was on for every device and is now off, with "Allow for <device>".

import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { listen } from "@tauri-apps/api/event";
import { commands, type PeerPermissions, type PermissionNotice } from "../bindings";
import { peerName, state, unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { toast } from "./ui";
import { t, type Key } from "../i18n";
import { HelpLink } from "../help/HelpLink";

export type Perm = "control" | "screen" | "kvm";
const PERMS: Perm[] = ["control", "screen", "kvm"];

/** What `p` allows, in words ("control, see the screen"); "" for nothing. */
export function permWords(p: PeerPermissions): string {
  return PERMS.filter((k) => p[k])
    .map((k) => t(`perm.name.${k}` as Key))
    .join(", ");
}

/** Your other computers that can share mouse and keyboard with this one. */
async function kvmComputers(): Promise<string[]> {
  try {
    const s = await commands.kvmSettings();
    return s.supported ? s.computers.map((c) => c.node_id) : [];
  } catch {
    return [];
  }
}

async function setPerm(node: string, perm: Perm, on: boolean): Promise<PeerPermissions | null> {
  try {
    return await unwrap(commands.setPeerPermission(node, perm, on));
  } catch (e) {
    toast(friendlyError(e), "error");
    return null;
  }
}

/** A device page's Permissions: one switch each, off until allowed. */
export function PermissionsTile(props: { node: string }) {
  const [p, setP] = createSignal<PeerPermissions>();
  const [kvm, setKvm] = createSignal(false);
  const load = () => {
    commands.peerPermissions(props.node).then(setP, () => {});
    kvmComputers().then((list) => setKvm(list.includes(props.node)));
  };
  createEffect(on(() => props.node, load));
  onMount(() => {
    const off = listen<{ node_id: string }>("permissions-changed", (e) => {
      if (e.payload.node_id === props.node) load();
    });
    onCleanup(() => off.then((u) => u()));
  });
  const name = () => peerName(props.node);
  const row = (perm: Perm) => (
    <>
      <label class="switch-row">
        <span>{t(`perm.${perm}` as Key, { name: name() })}</span>
        <input
          type="checkbox"
          role="switch"
          checked={!!p()?.[perm]}
          onChange={async (e) => {
            const want = e.currentTarget.checked;
            const got = await setPerm(props.node, perm, want);
            if (got) setP(got);
            else e.currentTarget.checked = !want;
          }}
        />
      </label>
      <p class="muted small">{t(`perm.${perm}Hint` as Key)}</p>
    </>
  );
  return (
    <article class="tile" id="device-permissions">
      <h2>
        {t("perm.title")} <HelpLink topic="privacy" />
      </h2>
      <p class="muted small">{t("perm.intro", { name: name() })}</p>
      {row("control")}
      {row("screen")}
      <Show when={kvm()}>{row("kvm")}</Show>
    </article>
  );
}

/** Settings: who may do what here. */
export function PermissionsSummary() {
  const [all, setAll] = createSignal<PeerPermissions[]>([]);
  const load = () => commands.allPermissions().then(setAll, () => {});
  onMount(() => {
    load();
    const off = listen("permissions-changed", load);
    onCleanup(() => off.then((u) => u()));
  });
  return (
    <div class="perm-summary" id="settings-permissions">
      <h3 class="small">
        {t("perm.summaryTitle")} <HelpLink topic="privacy" />
      </h3>
      <Show when={all().length > 0} fallback={<p class="muted small">{t("perm.summaryNone")}</p>}>
        <ul class="small">
          <For each={all()}>{(p) => <li>{t("perm.summaryLine", { name: peerName(p.node_id), what: permWords(p) })}</li>}</For>
        </ul>
      </Show>
      <p class="muted small">{t("perm.summaryHint")}</p>
    </div>
  );
}

type Ask = { from: string; perm: Perm };

/** "<device> wants to …": Allow once / Always / Don't allow. One at a time;
 *  more wait their turn. And a refusal from another device, as a toast. */
export function PermissionAsk() {
  const [queue, setQueue] = createSignal<Ask[]>([]);
  const ask = () => queue()[0];
  const offs: Promise<() => void>[] = [];
  onMount(() => {
    offs.push(
      listen<Ask>("permission-ask", (e) => {
        const a = e.payload;
        if (!PERMS.includes(a.perm)) return;
        setQueue((q) => (q.some((x) => x.from === a.from && x.perm === a.perm) ? q : [...q, a]));
      }),
    );
    offs.push(
      listen<{ from: string; perm: Perm; why: string }>("permission-refused", (e) => {
        const { from, perm } = e.payload;
        if (!PERMS.includes(perm)) return;
        toast(t(`perm.refused.${perm}` as Key, { name: peerName(from) }), "error");
      }),
    );
  });
  onCleanup(() => offs.forEach((p) => p.then((u) => u()).catch(() => {})));

  const answer = async (a: Ask, how: "once" | "always" | "deny" | null) => {
    setQueue((q) => q.slice(1));
    if (!how) return;
    try {
      await unwrap(commands.permissionAnswer(a.from, a.perm, how));
    } catch (e) {
      toast(friendlyError(e), "error");
    }
  };

  return (
    <Show when={ask()}>
      {(a) => (
        <Portal>
          <div class="send-to-backdrop">
            <div
              class="send-to"
              role="alertdialog"
              aria-modal="true"
              aria-labelledby="perm-ask-title"
              aria-describedby="perm-ask-body"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  answer(a(), null);
                }
              }}
            >
              <h2 id="perm-ask-title">{t(`perm.ask.${a().perm}` as Key, { name: peerName(a().from) })}</h2>
              <p id="perm-ask-body" class="muted small">
                {t("perm.askBody")}
              </p>
              <div class="row">
                <button class="btn btn-ghost" type="button" ref={(el) => setTimeout(() => el.focus())} onClick={() => answer(a(), "deny")}>
                  {t("perm.deny")}
                </button>
                <button class="btn" type="button" onClick={() => answer(a(), "once")}>
                  {t("perm.once")}
                </button>
                <button class="btn btn-primary" type="button" onClick={() => answer(a(), "always")}>
                  {t("perm.always")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
}

/** Once after the upgrade: these were on for all your devices and are now
 *  off; allow them again device by device, or not. */
export function PermissionNotice() {
  const [notice, setNotice] = createSignal<PermissionNotice | null>(null);
  const [kvm, setKvm] = createSignal<string[]>([]);
  const [done, setDone] = createSignal<Record<string, boolean>>({});
  onMount(async () => {
    const n = await commands.permissionNotice().catch(() => null);
    if (n && n.features.length) {
      setKvm(await kvmComputers());
      setNotice(n);
    }
  });
  const devices = (perm: string) => {
    const all = state.peers.filter((p) => !p.node_id.startsWith("manual:")).map((p) => p.node_id);
    return perm === "kvm" ? all.filter((n) => kvm().includes(n)) : all;
  };
  const allow = async (node: string, perm: Perm) => {
    if (await setPerm(node, perm, true)) setDone((d) => ({ ...d, [`${perm}|${node}`]: true }));
  };
  const close = () => {
    setNotice(null);
    commands.dismissPermissionNotice().catch(() => {});
  };
  return (
    <Show when={notice()}>
      {(n) => (
        <Portal>
          <div class="send-to-backdrop">
            <div
              class="send-to"
              role="dialog"
              aria-modal="true"
              aria-labelledby="perm-notice-title"
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  close();
                }
              }}
            >
              <h2 id="perm-notice-title">
                {t("perm.noticeTitle")} <HelpLink topic="privacy" />
              </h2>
              <p class="muted small">{t("perm.noticeBody")}</p>
              <For each={n().features.filter((f): f is Perm => PERMS.includes(f as Perm))}>
                {(perm) => (
                  <section>
                    <h3 class="small">{t(`perm.feature.${perm}` as Key)}</h3>
                    <Show when={devices(perm).length > 0} fallback={<p class="muted small">{t("perm.noDevices")}</p>}>
                      <div class="row wrap">
                        <For each={devices(perm)}>
                          {(node) => (
                            <button
                              class="btn btn-ghost btn-sm"
                              type="button"
                              disabled={!!done()[`${perm}|${node}`]}
                              onClick={() => allow(node, perm)}
                            >
                              {done()[`${perm}|${node}`]
                                ? t("perm.allowedFor", { name: peerName(node) })
                                : t("perm.allowFor", { name: peerName(node) })}
                            </button>
                          )}
                        </For>
                      </div>
                    </Show>
                  </section>
                )}
              </For>
              <div class="row">
                <button class="btn btn-primary" type="button" ref={(el) => setTimeout(() => el.focus())} onClick={close}>
                  {t("perm.noticeDone")}
                </button>
              </div>
            </div>
          </div>
        </Portal>
      )}
    </Show>
  );
}
