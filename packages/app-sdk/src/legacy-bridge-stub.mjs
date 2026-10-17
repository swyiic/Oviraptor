// Shared legacy-bridge adapter stub for Node workers.
// Vendored into installed packages as sdk/legacy-bridge-stub.mjs when vendorSdk is true.
// The app worker has no child-process grant. A legacy read port must be
// supplied by a trusted composition root; installed packages cannot create it.
export async function legacyRead(adapter,appId,name,args,context){
 if(typeof adapter!=='function')return{available:false,code:'legacy_managed_bridge_unavailable',executionAvailable:false};
 return await adapter({appId,name,arguments:args,context:{...context}});
}
