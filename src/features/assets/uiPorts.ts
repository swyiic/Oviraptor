/** Desktop implementation of Assets presentation ports. The standalone app
 * binds this owner-local module to its opaque-frame implementation at build. */
import { invoke } from "@tauri-apps/api/core";
import { confirmInPlace } from "../../shared/confirmInPlace";
import { useI18n as useAppI18n } from "../../i18n";
export { invoke };
export { openUrl as openAssetUrl } from "@tauri-apps/plugin-opener";
export { useI18n } from "../../i18n";

export const assetLocalStorage = {
  getItem: (key: string) => localStorage.getItem(key),
  setItem: (key: string, value: string) => localStorage.setItem(key, value),
  removeItem: (key: string) => localStorage.removeItem(key),
};
export const assetSessionStorage = {
  getItem: (key: string) => sessionStorage.getItem(key),
  setItem: (key: string, value: string) => sessionStorage.setItem(key, value),
  removeItem: (key: string) => sessionStorage.removeItem(key),
};
export async function assetConfirm(message: string): Promise<boolean> {
  // Confirms in place next to the control that asked (no native dialog). The
  // first paragraph is the question; the rest is detail.
  const [title, ...rest] = message.split(/\n\s*\n/);
  const { t } = useAppI18n();
  return confirmInPlace({ title: title.trim(), detail: rest.join("\n").trim(), confirmText: t.value.common.confirm, tone: "warning" });
}
export async function assetPrompt(message: string, initial = ""): Promise<string | null> { return window.prompt(message, initial); }
