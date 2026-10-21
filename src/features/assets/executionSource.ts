import type { AssetExecutionProfile } from "./types";

/** Internal compatibility ID only. Match Rust's stable default selection. */
export function defaultAssetExecutionProfile(profiles: readonly AssetExecutionProfile[]): AssetExecutionProfile | undefined {
  return [...profiles].sort((left, right) => Number(right.isDefault) - Number(left.isDefault) || left.id - right.id)[0];
}
