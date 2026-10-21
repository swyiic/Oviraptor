// A checked box is a disclosure for one observed recipient and grant revision.
// It must not become consent for whichever recipient happens to be current later.
export function snapshotDisclosure(runtimeIdentity,grantRevision){if(typeof runtimeIdentity!=='string'||!runtimeIdentity||!Number.isSafeInteger(grantRevision)||grantRevision<0)return null;return Object.freeze({accepted:true,runtimeIdentity,grantRevision});}
export function matchesDisclosure(value,runtimeIdentity,grantRevision){return Boolean(value?.accepted===true&&typeof runtimeIdentity==='string'&&runtimeIdentity&&value.runtimeIdentity===runtimeIdentity&&value.grantRevision===grantRevision);}
