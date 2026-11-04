// Keyboard helpers shared by the dialogs and inline forms: what can take
// focus inside an element, keeping Tab inside a modal, and handing focus
// back to where it was when something closes (docs/accessibility.md).

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** The elements in [root] that Tab reaches, in order (hidden ones left out). */
export function focusables(root: Element | undefined | null): HTMLElement[] {
  if (!root) return [];
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) => !el.hasAttribute("disabled") && el.getClientRects().length > 0,
  );
}

/** In a modal's keydown: Tab and Shift+Tab wrap around inside [root]. */
export function trapTab(e: KeyboardEvent, root: Element | undefined | null) {
  if (e.key !== "Tab") return;
  const list = focusables(root);
  if (!list.length) {
    e.preventDefault();
    return;
  }
  const first = list[0];
  const last = list[list.length - 1];
  const at = document.activeElement;
  if (e.shiftKey && (at === first || !root?.contains(at))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (at === last || !root?.contains(at))) {
    e.preventDefault();
    first.focus();
  }
}

/** Remembers what had focus now; the returned function puts it back (when
 *  that element is still on the page). */
export function rememberFocus(): () => void {
  const was = document.activeElement as HTMLElement | null;
  return () => {
    if (was && was.isConnected && typeof was.focus === "function") was.focus();
  };
}

/** Focus [el] on the next frame (once Solid has put it on the page). */
export function focusSoon(el: () => HTMLElement | undefined | null) {
  requestAnimationFrame(() => el()?.focus());
}
