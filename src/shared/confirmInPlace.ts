import { createApp, h, ref, type App } from "vue";
import InlineConfirm from "./components/InlineConfirm.vue";

// Destructive or irreversible actions confirm in place: the clicked button
// group is swapped for an InlineConfirm strip at the same spot, in the page
// flow. No modal, backdrop or native dialog.

export interface ConfirmInPlaceOptions {
  title: string;
  detail?: string;
  confirmText?: string;
  busyText?: string;
  cancelText?: string;
  tone?: "danger" | "warning" | "secondary";
  /** The control that asked; its button group is replaced while confirming.
   *  Defaults to the control the user just clicked or focused. */
  trigger?: Element | null;
  /** Insert the strip right after this element instead (nothing is hidden). */
  after?: Element | null;
  /** Runs on confirm while the strip shows its busy state. */
  run?: () => Promise<unknown> | unknown;
}

const CONTROL = 'button, a[href], [role="button"], [role="menuitem"], .button';
let lastPointer: { target: Element; at: number } | undefined;
if (typeof document !== "undefined") {
  document.addEventListener("pointerdown", (event) => {
    if (event.target instanceof Element) lastPointer = { target: event.target, at: Date.now() };
  }, true);
}

/** The control behind the action being confirmed: the one just clicked, or
 *  the focused one for keyboard activation. */
export function actionTrigger(): Element | null {
  // A recent click wins: WebKit does not move focus to clicked buttons, so the
  // focused element can be stale.
  if (lastPointer && Date.now() - lastPointer.at < 2000 && lastPointer.target.isConnected)
    return lastPointer.target.closest(CONTROL) || lastPointer.target;
  const focused = document.activeElement;
  if (focused instanceof Element && focused !== document.body && focused.closest(CONTROL)) return focused.closest(CONTROL);
  return null;
}

// A row of buttons is replaced as a whole; a button sitting among other
// content is replaced alone.
function groupOf(trigger: Element): Element {
  const parent = trigger.parentElement;
  if (parent && parent.children.length <= 8 && !parent.matches("body, main, .content-area")
    && [...parent.children].every((child) => child.matches(`${CONTROL}, input[type="checkbox"]`))) return parent;
  return trigger;
}

let active: { settle: (value: boolean) => void } | undefined;

export function confirmInPlace(options: ConfirmInPlaceOptions): Promise<boolean> {
  active?.settle(false);
  const trigger = options.after ? null : (options.trigger === undefined ? actionTrigger() : options.trigger);
  const group = trigger?.isConnected ? groupOf(trigger.closest(CONTROL) || trigger) : null;
  const anchor = options.after?.isConnected ? options.after : group;
  const host = document.createElement("div");
  host.className = "inline-confirm-host";
  // Clicks inside the strip must not reach a clickable row around it.
  host.addEventListener("click", (event) => event.stopPropagation());
  if (anchor) anchor.after(host);
  else (document.querySelector(".content-area") || document.body).prepend(host);
  group?.setAttribute("data-inline-confirming", "");

  const busy = ref(false);
  let app: App | undefined;
  let observer: MutationObserver | undefined;
  let done = false;
  return new Promise<boolean>((resolve) => {
    const settle = (value: boolean) => {
      if (done) return;
      done = true;
      if (active?.settle === settle) active = undefined;
      observer?.disconnect();
      document.removeEventListener("keydown", onKey, true);
      app?.unmount();
      host.remove();
      group?.removeAttribute("data-inline-confirming");
      const focusBack = (trigger?.closest(CONTROL) || trigger) as HTMLElement | null;
      if (focusBack?.isConnected) focusBack.focus({ preventScroll: true });
      resolve(value);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy.value) { event.stopPropagation(); settle(false); }
    };
    const confirm = async () => {
      if (busy.value) return;
      if (!options.run) { settle(true); return; }
      busy.value = true;
      try { await options.run(); } catch (error) { console.error(error); } finally { settle(true); }
    };
    active = { settle };
    app = createApp({
      render: () => h(InlineConfirm, {
        title: options.title,
        detail: options.detail || "",
        confirmText: options.confirmText,
        busyText: options.busyText,
        cancelText: options.cancelText,
        tone: options.tone || "danger",
        busy: busy.value,
        onConfirm: confirm,
        onCancel: () => { if (!busy.value) settle(false); },
      }),
    });
    app.mount(host);
    document.addEventListener("keydown", onKey, true);
    // The row it sits in can be re-rendered away (list refresh, navigation).
    observer = new MutationObserver(() => { if (!host.isConnected) settle(false); });
    observer.observe(document.body, { childList: true, subtree: true });
    const cancelButton = host.querySelector<HTMLElement>(".inline-confirm button");
    cancelButton?.focus({ preventScroll: true });
    host.scrollIntoView({ block: "nearest" });
  });
}
