// A configuration visit may unmount the page. Keep its unsent form in memory;
// it neither changes saved settings nor starts work when the page returns.
export type DiscoveryPrimaryDraft={organization:string;includeIsolated:boolean;collectFirst:boolean;collectionTargets:string;allowHunter:boolean;editingTarget:boolean;showSettings:boolean};
const drafts=new Map<number,{expiresAt:number;value:DiscoveryPrimaryDraft}>();
export function rememberDiscoveryPrimaryDraft(projectId:number,value:DiscoveryPrimaryDraft){
 if(!Number.isSafeInteger(projectId)||projectId<1)return;
 drafts.delete(projectId);
 drafts.set(projectId,{expiresAt:Date.now()+30*60*1000,value:{...value}});
 while(drafts.size>20)drafts.delete(drafts.keys().next().value!);
}
export function takeDiscoveryPrimaryDraft(projectId:number):DiscoveryPrimaryDraft|undefined{
 const draft=drafts.get(projectId);drafts.delete(projectId);
 return draft&&draft.expiresAt>Date.now()?{...draft.value}:undefined;
}
