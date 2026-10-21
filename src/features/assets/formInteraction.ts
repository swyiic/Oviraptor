/** Keep normal input Enter submission when an opaque app cannot use HTML forms.
 * Buttons retain their native click/keyboard activation; multiline, selection,
 * composition and modified-key input remain owned by the respective control. */
export function submitAssetsFormOnEnter(event: KeyboardEvent, submit: () => unknown, disabled = false): void {
  if (disabled || event.key !== "Enter" || event.defaultPrevented || event.isComposing || event.repeat
    || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const target = event.target as HTMLInputElement | null;
  if (target?.tagName !== "INPUT" || target.disabled || target.readOnly
    || !["text", "search", "email", "url", "tel", "password", "number"].includes(target.type)) return;
  event.preventDefault();
  void submit();
}
