import type { ConfigPolicyReference, ConfigProfileSection } from "./configProfileSections";
import { cloneConfigProfileSettings, configPolicyPreviewSettings } from "./configProfileSections";
import type { CategoryPendingWrite, ConfigurationCategory } from "./categoryTypes";

export function categoryReferenceKey(reference: ConfigPolicyReference | null | undefined): string {
  return reference ? `${reference.policyId}:${reference.revision}` : "builtin";
}
export function sameCategoryReference(left: ConfigPolicyReference | null | undefined, right: ConfigPolicyReference | null | undefined): boolean {
  return categoryReferenceKey(left) === categoryReferenceKey(right) && left?.kind === right?.kind;
}
export function categoryReferenceIsHead(view: ConfigurationCategory): boolean {
  return !view.reference || view.entries.some(entry => sameCategoryReference(entry.reference, view.reference));
}

/** A historical source contributes only this category's supported values. */
export function legacyCategoryDraft(view: ConfigurationCategory, source: Record<string, unknown>): Record<string, unknown> {
  const settings = { ...cloneConfigProfileSettings(view.builtinSettings), ...configPolicyPreviewSettings(source, view.section) };
  return cloneConfigProfileSettings(configPolicyPreviewSettings(settings, view.section));
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export type CategoryRecoveryStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;
function recoveryKey(section: ConfigProfileSection) { return `oviraptor.configuration.categoryRequest.${section}`; }
export function categoryRecoveryId(storage: CategoryRecoveryStorage, section: ConfigProfileSection): string | undefined {
  const value = storage.getItem(recoveryKey(section));
  if (value && !UUID.test(value)) throw new Error("configuration_category_recovery_id_invalid");
  return value ?? undefined;
}
export function prepareCategoryWrite(storage: CategoryRecoveryStorage, pending: CategoryPendingWrite): CategoryPendingWrite {
  const result = cloneConfigProfileSettings(pending);
  const prior = categoryRecoveryId(storage, result.input.section);
  result.input.requestId = prior ?? result.input.requestId;
  if (!UUID.test(result.input.requestId)) throw new Error("configuration_category_request_id_invalid");
  // Persist an acknowledgement id only. The full request and credentials stay in memory.
  storage.setItem(recoveryKey(result.input.section), result.input.requestId);
  return result;
}
export function acknowledgeCategoryWrite(storage: CategoryRecoveryStorage, section: ConfigProfileSection, requestId: string): void {
  const prior = categoryRecoveryId(storage, section);
  if (prior && prior !== requestId) throw new Error("configuration_category_receipt_mismatch");
  if (prior) storage.removeItem(recoveryKey(section));
}
