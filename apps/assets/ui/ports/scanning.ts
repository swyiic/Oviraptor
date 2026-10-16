/** No Scanning implementation or shared IDs enter the Assets package. The
 * receipt-based cross-app handoff is connected only after its explicit contract. */
export const sentinelApi={async createSentinelScan(){throw Object.assign(new Error('assets_scanning_handoff_not_connected'),{code:'assets_scanning_handoff_not_connected'});}};
