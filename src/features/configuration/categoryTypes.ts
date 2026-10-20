import type { ConfigPolicyReference, ConfigProfileSection } from "./configProfileSections";

export type CategoryDefaultSelection = { revision: number; reference: ConfigPolicyReference | null };
export type CategoryCatalogEntry = { name: string; reference: ConfigPolicyReference; versions: ConfigPolicyReference[] };
export type ConfigurationCategory = {
  section: ConfigProfileSection;
  source: "legacy-fallback" | "builtin" | "owner";
  legacyProfileId: number | null;
  defaultSelection: CategoryDefaultSelection | null;
  reference: ConfigPolicyReference | null;
  settings: Record<string, unknown>;
  builtinSettings: Record<string, unknown>;
  entries: CategoryCatalogEntry[];
  legacyAvailable: boolean;
};
export type SaveCategoryInput = {
  requestId: string; section: ConfigProfileSection; mode: "create" | "revise" | "fork";
  name: string; settings: Record<string, unknown>; expectedSettings: Record<string, unknown>;
  expectedReference: ConfigPolicyReference | null;
  expectedDefault: CategoryDefaultSelection | null; makeDefault: boolean;
};
export type SelectCategoryDefaultInput = {
  requestId: string; section: ConfigProfileSection; reference: ConfigPolicyReference | null;
  expectedDefault: CategoryDefaultSelection | null;
};
export type CategoryWriteResult = {
  receipt: {
    schemaVersion: 1; requestId: string; section: ConfigProfileSection;
    action: "create" | "revise" | "fork" | "select";
    reference: ConfigPolicyReference | null;
    defaultSelection: CategoryDefaultSelection | null;
  };
  currentDefault: CategoryDefaultSelection | null;
  /** Compares the current default selection with the original acknowledgement. */
  currentState: "unchanged" | "modified";
};
export type CategoryPendingWrite = { command: "save"; input: SaveCategoryInput } | { command: "select"; input: SelectCategoryDefaultInput };
