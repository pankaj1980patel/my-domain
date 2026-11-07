// Settings → Account: change the account password, sign out all devices,
// and the signed-in sessions (each with "Sign out"). And over everything:
// the server signed this computer out (its password was changed or it was
// signed out from another device): ask for the password again instead of
// failing silently (`SignedOutPrompt`, on the core's `signed-out` event).
//
// The account password (the server's) is not the encryption passphrase
// (`KeyRotation.tsx`); each says so in one line.

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { commands, type AccountSession } from "../bindings";
import { unwrap } from "../lib/core";
import { friendlyError, niceName, shortDate, timeAgo } from "../lib/words";
import { rememberFocus, trapTab } from "../lib/a11y";
import { toast } from "./ui";
import { SettingRow } from "./SettingRow";
import { t, type Key } from "../i18n";

const STRENGTH = ["keyRotation.strength0", "keyRotation.strength1", "keyRotation.strength2", "keyRotation.strength3", "keyRotation.strength4"] as const;

/** A modal confirm: Cancel first (never the default), then the action. */
function ConfirmDialog(props: {
  id: string;
  title: string;
  children: JSX.Element;
  action: string;
  busy?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  let dialog: HTMLDivElement | undefined;
  const restore = rememberFocus();
  const close = () => {
    props.onClose();
    restore();
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <div
          class="clip-pic-preview key-confirm"
          ref={(el) => {
            dialog = el;
            queueMicrotask(() => el.querySelector<HTMLElement>("input, button")?.focus());
          }}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby={`${props.id}-title`}
          aria-describedby={`${props.id}-body`}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.preventDefault();
              close();
            } else trapTab(e, dialog);
          }}
        >
          <h2 id={`${props.id}-title`}>{props.title}</h2>
          <div id={`${props.id}-body`}>{props.children}</div>
          <div class="row">
            <button class="btn btn-ghost btn-sm" onClick={close}>
              {t("settings.cancel")}
            </button>
            <span class="grow" />
            <button class="btn btn-primary btn-sm" disabled={props.busy} onClick={props.onConfirm}>
              {props.action}
            </button>
          </div>
        </div>
      </div>
    </Portal>
  );
}

/** "Change account password": the form, then a confirm saying what happens. */
export function ChangePassword() {
  const [open, setOpen] = createSignal(false);
  const [current, setCurrent] = createSignal("");
  const [next, setNext] = createSignal("");
  const [repeat, setRepeat] = createSignal("");
  const [strength, setStrength] = createSignal(0);
  const [problem, setProblem] = createSignal<string | null>(null);
  const [confirming, setConfirming] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const check = () =>
    commands.accountPasswordProblem(current(), next(), repeat()).then(setProblem, () => setProblem(null));
  const typingNew = (v: string) => {
    setNext(v);
    commands.passphraseStrength(v).then(setStrength).catch(() => {});
    check();
  };
  const reset = () => {
    setOpen(false);
    setConfirming(false);
    setCurrent("");
    setNext("");
    setRepeat("");
    setStrength(0);
    setProblem(null);
  };
  const submit = async (e: Event) => {
    e.preventDefault();
    const p = await commands.accountPasswordProblem(current(), next(), repeat()).catch(() => null);
    setProblem(p);
    if (!p) setConfirming(true);
  };
  const change = async () => {
    setBusy(true);
    try {
      await unwrap(commands.changeAccountPassword(current(), next()));
      reset();
      toast(t("account.changed"), "ok");
    } catch (err) {
      const m = String(err instanceof Error ? err.message : err).toLowerCase();
      setConfirming(false);
      toast(m.includes("incorrect password") ? t("account.wrongPassword") : friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };
  // Only once the repeat is typed (or on Continue) is a mismatch shown.
  const shown = () => {
    const p = problem();
    if (!p) return null;
    if (p === "account.mismatch" && !repeat()) return null;
    if (p === "account.needCurrent" && !next()) return null;
    if (p === "account.tooShort" && !next()) return null;
    return p as Key;
  };
  return (
    <>
      <SettingRow icon="key" label={t("account.passwordTitle")} hint={t("account.passwordHint")}>
        <Show when={!open()}>
          <button class="btn btn-soft btn-sm" onClick={() => setOpen(true)}>
            {t("account.change")}
          </button>
        </Show>
      </SettingRow>
      <Show when={open()}>
        <form class="stack" onSubmit={submit}>
          <input
            type="password"
            autocomplete="current-password"
            placeholder={t("account.current")}
            aria-label={t("account.current")}
            value={current()}
            onInput={(e) => {
              setCurrent(e.currentTarget.value);
              check();
            }}
          />
          <input
            type="password"
            autocomplete="new-password"
            placeholder={t("account.new")}
            aria-label={t("account.new")}
            value={next()}
            onInput={(e) => typingNew(e.currentTarget.value)}
          />
          <input
            type="password"
            autocomplete="new-password"
            placeholder={t("account.repeat")}
            aria-label={t("account.repeat")}
            value={repeat()}
            onInput={(e) => {
              setRepeat(e.currentTarget.value);
              check();
            }}
          />
          <Show when={next()}>
            <p class="small muted" aria-live="polite">
              {t("keyRotation.strength", { level: t(STRENGTH[strength()] ?? STRENGTH[0]) })}
            </p>
          </Show>
          <Show when={shown()}>{(k) => <p class="small" role="alert">{t(k())}</p>}</Show>
          <div class="row">
            <button class="btn btn-primary" type="submit" disabled={busy()}>
              {t("account.continue")}
            </button>
            <button class="btn btn-ghost" type="button" onClick={reset}>
              {t("settings.cancel")}
            </button>
          </div>
        </form>
      </Show>
      <Show when={confirming()}>
        <ConfirmDialog
          id="change-password"
          title={t("account.confirmTitle")}
          action={t("account.confirmButton")}
          busy={busy()}
          onConfirm={change}
          onClose={() => setConfirming(false)}
        >
          <p class="small">{t("account.confirmBody")}</p>
          <p class="small muted">{t("account.confirmKey")}</p>
        </ConfirmDialog>
      </Show>
    </>
  );
}

/** "Sign out all devices", with a confirm (and the password, to stay in here). */
export function SignOutAll() {
  const [asking, setAsking] = createSignal(false);
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const go = async () => {
    setBusy(true);
    try {
      const back = await unwrap(commands.signOutAllDevices(password()));
      setAsking(false);
      setPassword("");
      // Not signed back in: the `signed-out` event asks for the password.
      toast(back ? t("account.signedOutAll") : t("account.signedOutAllHere"), back ? "ok" : "info");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <SettingRow icon="disconnect" label={t("account.signOutAll")} hint={t("account.signOutAllHint")}>
        <button class="btn btn-danger btn-sm" onClick={() => setAsking(true)}>
          {t("account.signOutAll")}
        </button>
      </SettingRow>
      <Show when={asking()}>
        <ConfirmDialog
          id="sign-out-all"
          title={t("account.signOutAllTitle")}
          action={t("account.signOutAllButton")}
          busy={busy()}
          onConfirm={go}
          onClose={() => {
            setAsking(false);
            setPassword("");
          }}
        >
          <p class="small">{t("account.signOutAllBody")}</p>
          <input
            type="password"
            autocomplete="current-password"
            placeholder={t("account.signOutAllPassword")}
            aria-label={t("account.signOutAllPassword")}
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && go()}
          />
          <p class="small muted">{t("account.signOutAllPasswordHint")}</p>
        </ConfirmDialog>
      </Show>
    </>
  );
}

function sessionName(s: AccountSession): string {
  if (s.current) return t("account.thisDevice");
  return s.name ? niceName(s.name) : t("account.unknownDevice");
}

/** The account's signed-in sessions, each with "Sign out". */
export function Sessions() {
  const [list, setList] = createSignal<AccountSession[] | null>(null);
  const [note, setNote] = createSignal("");
  const [asking, setAsking] = createSignal<AccountSession | null>(null);
  const [busy, setBusy] = createSignal(false);
  const load = async () => {
    try {
      setList(await unwrap(commands.accountSessions()));
      setNote("");
    } catch (err) {
      setList([]);
      setNote(friendlyError(err));
    }
  };
  onMount(() => void load());
  const signOut = async (s: AccountSession) => {
    setBusy(true);
    try {
      await unwrap(commands.signOutSession(s.id));
      toast(t("account.sessionSignedOut", { name: sessionName(s) }), "ok");
    } catch (err) {
      toast(friendlyError(err), "error");
    } finally {
      setBusy(false);
      setAsking(null);
      await load();
    }
  };
  return (
    <>
      <h3>{t("account.sessionsTitle")}</h3>
      <p class="muted small">{t("account.sessionsHint")}</p>
      <Show when={note()}>
        <p class="small muted">{note()}</p>
      </Show>
      <Show when={list()}>
        {(l) => (
          <Show when={l().length > 0} fallback={<Show when={!note()}><p class="muted small">{t("account.sessionsNone")}</p></Show>}>
            <div class="removed-list">
              <For each={l()}>
                {(s) => (
                  <div class="row removed-row">
                    <div class="grow">
                      <div>
                        <Show when={s.current} fallback={sessionName(s)}>
                          <strong>{sessionName(s)}</strong>
                        </Show>
                      </div>
                      <div class="small muted">
                        {t("account.sessionTimes", {
                          created: s.created == null ? "—" : shortDate(s.created * 1000),
                          seen: s.last_seen == null ? "—" : timeAgo(s.last_seen * 1000),
                        })}
                      </div>
                    </div>
                    <Show when={!s.current}>
                      <button class="btn btn-ghost btn-sm" disabled={busy()} onClick={() => setAsking(s)}>
                        {t("account.signOutSession")}
                      </button>
                    </Show>
                  </div>
                )}
              </For>
            </div>
          </Show>
        )}
      </Show>
      <button class="btn btn-ghost btn-sm" onClick={() => void load()}>
        {t("account.refresh")}
      </button>
      <Show when={asking()}>
        {(s) => (
          <ConfirmDialog
            id="sign-out-session"
            title={t("account.signOutSessionTitle", { name: sessionName(s()) })}
            action={t("account.signOutSession")}
            busy={busy()}
            onConfirm={() => void signOut(s())}
            onClose={() => setAsking(null)}
          >
            <p class="small">{t("account.signOutSessionBody")}</p>
          </ConfirmDialog>
        )}
      </Show>
    </>
  );
}

/** Settings → Account, under "Signed in as". */
export function AccountSecurity() {
  return (
    <div class="stack account-security">
      <ChangePassword />
      <SignOutAll />
      <Sessions />
    </div>
  );
}

/** The server signed this computer out: the password signs it back in;
 *  "Sign in another way" goes to the full sign-in screen. */
export function SignedOutPrompt(props: { username: string; onDone: () => void; onOther: () => void }) {
  const [password, setPassword] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let dialog: HTMLFormElement | undefined;
  const submit = async (e: Event) => {
    e.preventDefault();
    if (!password()) return;
    setBusy(true);
    setError("");
    try {
      await unwrap(commands.signInAgain(password()));
      setPassword("");
      toast(t("account.signedInAgain"), "ok");
      props.onDone();
    } catch (err) {
      const m = String(err instanceof Error ? err.message : err).toLowerCase();
      // No encryption key here any more: only a full sign-in helps.
      if (m.includes("encryption key")) return props.onOther();
      setError(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Portal>
      <div class="clip-pic-backdrop">
        <form
          class="clip-pic-preview key-confirm"
          ref={(el) => {
            dialog = el;
            queueMicrotask(() => el.querySelector<HTMLElement>("input")?.focus());
          }}
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="signed-out-title"
          aria-describedby="signed-out-body"
          onKeyDown={(e) => trapTab(e, dialog)}
          onSubmit={submit}
        >
          <h2 id="signed-out-title">{t("account.signedOutTitle")}</h2>
          <p id="signed-out-body" class="small">
            {t("account.signedOutBody", {
              name: props.username ? t("account.signedOutAs", { name: props.username }) : "",
            })}
          </p>
          <input
            type="password"
            autocomplete="current-password"
            placeholder={t("signIn.password")}
            aria-label={t("signIn.password")}
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
          />
          <Show when={error()}>
            <p class="small" role="alert">
              {error()}
            </p>
          </Show>
          <div class="row">
            <button class="btn btn-ghost btn-sm" type="button" onClick={props.onOther}>
              {t("account.signInOther")}
            </button>
            <span class="grow" />
            <button class="btn btn-primary btn-sm" type="submit" disabled={busy() || !password()}>
              {t("account.signInAgain")}
            </button>
          </div>
        </form>
      </div>
    </Portal>
  );
}
