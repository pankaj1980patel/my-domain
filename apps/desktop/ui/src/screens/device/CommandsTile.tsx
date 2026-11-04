// Another computer's commands: the ones its owner wrote in its Settings
// ("Commands my other devices can run"), listed in its status (`commands`).
// Run sends only the command's id (`run {id}`); the computer runs it and puts
// how it went in its status (`last_run`), which this tile asks for until it
// changes.

import { createSignal, For, onCleanup, Show } from "solid-js";
import { peerName, requestStatus, runAction, state } from "../../lib/core";
import { friendlyError, niceName } from "../../lib/words";
import { toast } from "../../components/ui";
import { t } from "../../i18n";

/** How often to ask for the result while one runs. */
const POLL_MS = 1500;
/** Give up waiting after this (the computer stops a command after 60 s). */
const WAIT_MS = 75_000;

export default function CommandsTile(props: { node: string }) {
  const [running, setRunning] = createSignal<string | null>(null);
  const name = () => niceName(peerName(props.node));
  const st = () => state.status[props.node];
  let timer: ReturnType<typeof setInterval> | undefined;
  const stop = () => {
    clearInterval(timer);
    timer = undefined;
    setRunning(null);
  };
  onCleanup(stop);

  const run = async (id: string, what: string) => {
    // The result is new once `last_run` changes from what it is now (its
    // `at` is the computer's clock, which may differ from this one's).
    const before = st()?.lastRun;
    setRunning(id);
    try {
      const via = await runAction(props.node, "run", { id });
      toast(via === "queued" ? t("run.queued", { name: name() }) : t("run.asked", { name: name(), what }));
    } catch (e) {
      toast(friendlyError(e), "error");
      stop();
      return;
    }
    const started = Date.now();
    clearInterval(timer);
    timer = setInterval(() => {
      const last = st()?.lastRun;
      if (last && last.id === id && (last.at !== before?.at || last.id !== before?.id)) {
        if (last.error) toast(t("run.refused", { name: name(), why: last.error }), "error");
        stop();
      } else if (Date.now() - started > WAIT_MS) {
        toast(t("run.noAnswer", { name: name() }));
        stop();
      } else {
        requestStatus(props.node).catch(() => {});
      }
    }, POLL_MS);
  };

  return (
    <article class="tile">
      <h2>{t("run.title")}</h2>
      <p class="muted small">{t("run.hint", { name: name() })}</p>
      <ul class="commands-list">
        <For each={st()?.commands ?? []}>
          {(c) => (
            <li class="row">
              <span class="grow">{c.name}</span>
              <button class="btn btn-ghost btn-sm" disabled={running() !== null} onClick={() => run(c.id, c.name)}>
                {running() === c.id ? t("run.running") : t("run.run")}
              </button>
            </li>
          )}
        </For>
      </ul>
      <Show when={!st()?.lastRun?.error && st()?.lastRun}>{(r) => <RunResult run={r()} />}</Show>
    </article>
  );
}

/** How a run went: the exit code, then the end of what it printed. */
export function RunResult(props: { run: { name: string; code?: number | null; output: string; timedOut: boolean } }) {
  const what = () => props.run.name;
  const line = () =>
    props.run.timedOut
      ? t("run.timedOut", { what: what() })
      : typeof props.run.code === "number"
        ? t("run.exitCode", { what: what(), code: props.run.code })
        : t("run.stopped", { what: what() });
  return (
    <div class="stack run-result">
      <span class="small" classList={{ "run-failed": props.run.code !== 0 }}>
        {line()}
      </span>
      <pre class="run-output">{props.run.output || t("run.noOutput")}</pre>
    </div>
  );
}
