import { computed, ref } from "vue";
import { zh, type Messages } from "./locales/zh";
import { en } from "./locales/en";

export type { Messages };

/** Supported languages. Adding one is a catalog file in src/locales plus an
 *  entry here; the language menu, number formats and <html lang> follow. */
export const LOCALES = {
  zh: { label: "中文", intl: "zh-CN", messages: zh as Messages }, // i18n-ignore: a language is named in itself
  en: { label: "English", intl: "en-US", messages: en },
} satisfies Record<string, { label: string; intl: string; messages: Messages }>;

export type Locale = keyof typeof LOCALES;
const isLocale = (value: unknown): value is Locale => typeof value === "string" && value in LOCALES;

if (localStorage.getItem("oviraptor-locale") === null && localStorage.getItem("asset-atlas-locale") !== null) {
  localStorage.setItem("oviraptor-locale", localStorage.getItem("asset-atlas-locale")!);
}
localStorage.removeItem("asset-atlas-locale");
const stored = localStorage.getItem("oviraptor-locale");
export const locale = ref<Locale>(isLocale(stored) ? stored : "zh");

function applyDocumentLanguage(value: Locale) {
  if (typeof document !== "undefined") document.documentElement.lang = LOCALES[value].intl;
}
applyDocumentLanguage(locale.value);

/** Fills `{name}` placeholders in a catalog value. */
export function fmt(template: string, params: Record<string, string | number>) {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => (key in params ? String(params[key]) : match));
}

/** Numbers in the current language: 1,234 / 77.4万 in Chinese, 77.4K in English. */
export function formatInteger(value: number) {
  return new Intl.NumberFormat(LOCALES[locale.value].intl).format(Number(value) || 0);
}
export function formatCompact(value: number) {
  return new Intl.NumberFormat(LOCALES[locale.value].intl, { notation: "compact", maximumFractionDigits: 1 })
    .format(Math.max(0, Number(value) || 0));
}

export function useI18n() {
  const t = computed(() => LOCALES[locale.value].messages);
  // Transitional inline pair for text not yet in the catalogs: Chinese for zh,
  // English for every other language until that text moves into a catalog.
  const tr = (zhText: string, enText: string) => locale.value === "zh" ? zhText : enText;
  const setLocale = (value: Locale) => {
    locale.value = value;
    localStorage.setItem("oviraptor-locale", value);
    applyDocumentLanguage(value);
  };
  return { locale, t, tr, setLocale, fmt };
}
