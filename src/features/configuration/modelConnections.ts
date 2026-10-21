import type { ConfigPolicyReference } from "./configProfileSections";
import { cloneConfigProfileSettings } from "./configProfileSections";
import type { ConfigurationCategory } from "./categoryTypes";
import type { AgentModelProfile } from "./configProfileDefaults";

export type ModelConnectionBinding = ConfigPolicyReference & { kind: "models"; modelProfileId: string };
export type ModelDirectoryConnection = {
  key: string; name: string; profile: AgentModelProfile;
  reference: ConfigPolicyReference | null;
  category: ConfigurationCategory; historical: boolean;
};
export function directoryConnections(category: ConfigurationCategory): ModelDirectoryConnection[] {
  const profiles = category.settings.modelProfiles;
  if (!Array.isArray(profiles)) return [];
  return profiles.filter((profile): profile is AgentModelProfile => !!profile && typeof profile === "object" && typeof profile.id === "string").map(profile => ({
    key: `${category.reference?.policyId ?? "legacy"}:${category.reference?.revision ?? 0}:${profile.id}`,
    name: String(profile.name || profile.llm || profile.id), profile: cloneConfigProfileSettings(profile),
    reference: category.reference ? { ...category.reference } : null, category,
    historical: category.source === "legacy-fallback",
  }));
}
export function connectionBinding(connection: Pick<ModelDirectoryConnection, "reference" | "profile">): ModelConnectionBinding | null {
  return connection.reference ? { ...connection.reference, kind: "models", modelProfileId: connection.profile.id } : null;
}
export function matchesModelBinding(binding: ModelConnectionBinding | null | undefined, connection: ModelDirectoryConnection): boolean {
  const expected = connectionBinding(connection);
  return !!binding && !!expected && binding.kind === "models" && binding.policyId === expected.policyId && binding.revision === expected.revision && binding.modelProfileId === expected.modelProfileId;
}
export function modelSettingsForConnection(base: Record<string, unknown>, profile: AgentModelProfile, selectForScan: boolean): Record<string, unknown> {
  const settings = cloneConfigProfileSettings(base);
  const profiles = Array.isArray(settings.modelProfiles) ? settings.modelProfiles as AgentModelProfile[] : [];
  const exists = profiles.some(item => item.id === profile.id);
  settings.modelProfiles = exists ? profiles.map(item => item.id === profile.id ? { ...profile } : item) : [...profiles, { ...profile }];
  const activeId = selectForScan ? profile.id : String(settings.activeModelProfileId || profile.id);
  settings.activeModelProfileId = activeId;
  const active = (settings.modelProfiles as AgentModelProfile[]).find(item => item.id === activeId);
  settings.modelName = active?.llm ?? ""; settings.modelDeployment = active?.deployment ?? "cloud";
  settings.modelApiBase = active?.apiBase ?? "";
  settings.modelApiKey = active?.deployment === "cloud" ? active.apiKey : "";
  settings.localApiKey = active?.deployment === "local" ? active.localApiKey : "";
  return settings;
}
