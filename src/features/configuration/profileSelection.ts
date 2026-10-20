import type { ConfigProfile } from "./types";

/** Matches Rust default_settings: explicit default first, then the lowest stable ID. */
export function orderedConfigProfiles(profiles: readonly ConfigProfile[]): ConfigProfile[] {
  return [...profiles].sort((left, right) => Number(right.isDefault) - Number(left.isDefault) || left.id - right.id);
}

export function defaultConfigProfile(profiles: readonly ConfigProfile[]): ConfigProfile | undefined {
  return orderedConfigProfiles(profiles)[0];
}

export function hasHackerOneCredentials(profile: ConfigProfile | undefined): boolean {
  const settings = profile?.settings;
  return typeof settings?.hackerOneUsername === "string" && Boolean(settings.hackerOneUsername.trim())
    && typeof settings?.hackerOneToken === "string" && Boolean(settings.hackerOneToken.trim());
}

/** UI options contain metadata and availability only; no credential enters the DOM. */
export function hackerOneProfileOptions(profiles: readonly ConfigProfile[]) {
  return orderedConfigProfiles(profiles).map(profile => ({ id: profile.id, name: profile.name, isDefault: profile.isDefault, configured: hasHackerOneCredentials(profile) }));
}

export function initialHackerOneProfileId(profiles: readonly ConfigProfile[]): number | undefined {
  return defaultConfigProfile(profiles.filter(hasHackerOneCredentials))?.id ?? defaultConfigProfile(profiles)?.id;
}
