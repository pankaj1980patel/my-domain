// "Commands my other devices can run": the named shell commands this
// computer's owner writes here, which your phone and other computers show as
// buttons (commands_run.rs). Add, edit, switch on or off, delete, and Test
// (runs it here and shows how it went).

import { createSignal, For, onMount, Show } from "solid-js";
import { commands, type RunOutcome_Serialize as RunOutcome, type ShellCommand } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError } from "../lib/words";
import { Gate, toast } from "./ui";
import { RunResult } from "../screens/device/CommandsTile";
import { t } from "../i18n";

export function ShellCommandsSetting() {
  const [list, setList] = createSignal<ShellCommand[]>([]);
  const [name, setName] = createSignal("");
  const [line, setLine] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  onMount(() => commands.listShellCommands().then(setList, () => {}));

  const replace = (c: ShellCommand) => setList((l) => l.map((x) => (x.id === c.id ? c : x)));
  const save = (c: ShellCommand) =>
    unwrap(commands.updateShellCommand(c)).then(
      (saved) => {
        replace(saved);
        return true;
      },
      (e) => {
        toast(friendlyError(e), "error");
        return false;
      },
    );

  const add = async (e: Event) => {
    e.preventDefault();
    if (!name().trim() || !line().trim()) return;
    setBusy(true);
    try {
      const c = await unwrap(commands.addShellCommand({ name: name(), command: line(), enabled: true }));
      setList((l) => [...l, c]);
      setName("");
      setLine("");
      toast(t("commands.added", { name: c.name }), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };

  const remove = (c: ShellCommand) =>
    commands.deleteShellCommand(c.id).then(() => setList((l) => l.filter((x) => x.id !== c.id)));

  return (
    <article class="tile">
      <h2>{t("commands.title")}</h2>
      <p class="muted small">{t("commands.hint")}</p>
      <Show when={list().length} fallback={<p class="muted small">{t("commands.empty")}</p>}>
        <ul class="commands-list">
          <For each={list()}>{(c) => <CommandRow cmd={c} onSave={save} onDelete={remove} />}</For>
        </ul>
      </Show>
      <form class="stack" onSubmit={add}>
        <input
          type="text"
          placeholder={t("commands.namePlaceholder")}
          aria-label={t("commands.name")}
          maxLength={60}
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <input
          type="text"
          class="command-line"
          placeholder={t("commands.commandPlaceholder")}
          aria-label={t("commands.command")}
          spellcheck={false}
          value={line()}
          onInput={(e) => setLine(e.currentTarget.value)}
        />
        <Gate reason={name().trim() && line().trim() ? undefined : t("commands.needBoth")}>
          <button class="btn btn-soft btn-block" type="submit" disabled={busy() || !name().trim() || !line().trim()}>
            {t("commands.add")}
          </button>
        </Gate>
      </form>
    </article>
  );
}

function CommandRow(props: {
  cmd: ShellCommand;
  onSave: (c: ShellCommand) => Promise<boolean>;
  onDelete: (c: ShellCommand) => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [name, setName] = createSignal("");
  const [line, setLine] = createSignal("");
  const [testing, setTesting] = createSignal(false);
  const [result, setResult] = createSignal<RunOutcome | null>(null);

  const edit = () => {
    setName(props.cmd.name);
    setLine(props.cmd.command);
    setEditing(true);
  };
  const submit = async (e: Event) => {
    e.preventDefault();
    if (await props.onSave({ ...props.cmd, name: name(), command: line() })) setEditing(false);
  };
  const test = async () => {
    setTesting(true);
    setResult(null);
    try {
      setResult(await unwrap(commands.testShellCommand(props.cmd.id)));
    } catch (e) {
      toast(friendlyError(e), "error");
    } finally {
      setTesting(false);
    }
  };

  return (
    <li class="stack command-row" classList={{ "command-off": !props.cmd.enabled }}>
      <Show
        when={editing()}
        fallback={
          <>
            <div class="row">
              <strong class="grow">{props.cmd.name}</strong>
              <input
                type="checkbox"
                role="switch"
                checked={props.cmd.enabled}
                aria-label={t("commands.enabledLabel", { name: props.cmd.name })}
                onChange={(e) => props.onSave({ ...props.cmd, enabled: e.currentTarget.checked })}
              />
            </div>
            <code class="command-line small">{props.cmd.command}</code>
            <div class="row">
              <button class="btn btn-ghost btn-sm" disabled={testing()} onClick={test}>
                {testing() ? t("commands.testing") : t("commands.test")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={edit}>
                {t("commands.edit")}
              </button>
              <button class="btn btn-ghost btn-sm" onClick={() => props.onDelete(props.cmd)}>
                {t("commands.delete")}
              </button>
            </div>
          </>
        }
      >
        <form class="stack" onSubmit={submit}>
          <input
            type="text"
            aria-label={t("commands.name")}
            maxLength={60}
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
          <input
            type="text"
            class="command-line"
            aria-label={t("commands.command")}
            spellcheck={false}
            value={line()}
            onInput={(e) => setLine(e.currentTarget.value)}
          />
          <div class="row">
            <button class="btn btn-primary btn-sm" type="submit" disabled={!name().trim() || !line().trim()}>
              {t("commands.save")}
            </button>
            <button class="btn btn-ghost btn-sm" type="button" onClick={() => setEditing(false)}>
              {t("commands.cancel")}
            </button>
          </div>
        </form>
      </Show>
      <Show when={result()}>
        {(r) => <RunResult run={{ name: props.cmd.name, code: r().code, output: r().output, timedOut: r().timed_out }} />}
      </Show>
    </li>
  );
}
