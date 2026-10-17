/** Configuration center section keys (2026-10-17 refactor). */
export const CONFIG_CENTER_SECTIONS = [
  "runtime-model",
  "apps",
  "data-permissions",
  "advanced",
] as const;

export type ConfigCenterSection = (typeof CONFIG_CENTER_SECTIONS)[number];

/** Legacy App.vue keys → new sections (deep links / managePlugins keep working). */
export const CONFIG_SECTION_ALIASES: Record<string, ConfigCenterSection> = {
  "agent-runtime": "runtime-model",
  profiles: "advanced",
  rules: "advanced",
  workers: "advanced",
  runtime: "advanced",
  plugins: "apps",
  "runtime-model": "runtime-model",
  apps: "apps",
  "data-permissions": "data-permissions",
  advanced: "advanced",
};

export function resolveConfigSection(value: string | null | undefined): ConfigCenterSection {
  if (!value) return "runtime-model";
  return CONFIG_SECTION_ALIASES[value] ?? "runtime-model";
}

export function isConfigCenterSection(value: string): value is ConfigCenterSection {
  return (CONFIG_CENTER_SECTIONS as readonly string[]).includes(value);
}

/** Advanced subsection when landing via a legacy alias. */
export type AdvancedSubSection = "profiles" | "rules" | "workers" | "runtime" | "legacy-models";

export function advancedSubSectionFromAlias(value: string | null | undefined): AdvancedSubSection {
  if (value === "rules") return "rules";
  if (value === "workers") return "workers";
  if (value === "runtime") return "runtime";
  if (value === "profiles") return "profiles";
  return "profiles";
}
