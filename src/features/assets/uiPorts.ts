/** Desktop implementation of Assets presentation ports. The standalone app
 * binds this owner-local module to its opaque-frame implementation at build. */
export { invoke } from "@tauri-apps/api/core";
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
export async function assetConfirm(message: string): Promise<boolean> { return window.confirm(message); }
export async function assetPrompt(message: string, initial = ""): Promise<string | null> { return window.prompt(message, initial); }
