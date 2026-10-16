/** Assets sends a provenance-bound selection for explicit review. A receipt is
 * neither a Scanning draft identity nor execution/authorization. */
import {handoffAssetsSelection} from './client.mjs';
export const sentinelApi={createSentinelScan:(projectId:number,assetIds:number[],mode:'quick'|'standard'|'deep'='standard')=>handoffAssetsSelection(projectId,assetIds,mode)};
