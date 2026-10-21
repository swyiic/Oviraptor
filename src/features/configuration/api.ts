import { invoke } from "@tauri-apps/api/core";
import type { ConfigProfile, ModelProfileTestResult, FofaApiTestResult } from "./types";
import type { ConfigPolicyReference, ConfigProfileSection } from "./configProfileSections";
import type { CategoryWriteResult, ConfigurationCategory, SaveCategoryInput, SelectCategoryDefaultInput } from "./categoryTypes";

type ProfileSectionInput = { profileId: number; section: ConfigProfileSection; settings: Record<string, unknown>; expectedSettings: Record<string, unknown>; expectedReference?: ConfigPolicyReference };

export const configurationApi = {
  getCategory: (section: ConfigProfileSection, reference?: ConfigPolicyReference) =>
    invoke<ConfigurationCategory>("get_configuration_category", { section, ...(reference ? { reference } : {}) }),
  saveCategory: (input: SaveCategoryInput) =>
    invoke<CategoryWriteResult>("save_configuration_category", { input }),
  setCategoryDefault: (input: SelectCategoryDefaultInput) =>
    invoke<CategoryWriteResult>("set_configuration_category_default", { input }),
  getCategoryReceipt: (requestId: string) =>
    invoke<CategoryWriteResult | null>("get_configuration_category_receipt", { requestId }),
  listProfiles: () => invoke<ConfigProfile[]>("list_config_profiles"),
  saveProfileSection: (input: ProfileSectionInput) =>
    invoke<number>("save_config_profile_section", { input }),
  adoptProfilePolicy: (input: ProfileSectionInput) =>
    invoke<number>("adopt_config_profile_policy", { input }),
  saveProfileMetadata: (input: { profileId: number; name: string; description: string; isDefault: boolean; expected: { name: string; description: string; isDefault: boolean } }) =>
    invoke<number>("save_config_profile_metadata", { input }),
  saveProfile: (input: {
    id?: number;
    name: string;
    description: string;
    isDefault: boolean;
    settings: Record<string, any>;
  }) => invoke<number>("save_config_profile", { input }),
  deleteProfile: (profileId: number) =>
    invoke<void>("delete_config_profile", { profileId }),
  testModelProfile: (input: {
    llm: string; deployment: "cloud" | "local"; apiBase: string; apiKey: string;
  }) => invoke<ModelProfileTestResult>("test_model_profile", { input }),
  testFofaApi: (input: { key: string; proxyUrl: string; email?: string }) =>
    invoke<FofaApiTestResult>("test_fofa_api", { input }),
};
