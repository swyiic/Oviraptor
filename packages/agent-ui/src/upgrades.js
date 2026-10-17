import {entityId,segment} from './api.js';
export function upgradeSnapshot(app,path){const directory=typeof path==='string'?path.trim():'';if(!directory||!entityId(app.id)||typeof app.version!=='string'||!app.version||app.enabled||app.busy||!Number.isSafeInteger(app.revision)||app.revision<1)return null;return Object.freeze({id:entityId(app.id),version:app.version,revision:app.revision,path:directory});}
export function matchesUpgrade(snapshot,app,path){return Boolean(snapshot&&entityId(app.id)===snapshot.id&&app.version===snapshot.version&&app.revision===snapshot.revision&&!app.enabled&&!app.busy&&snapshot.path===path.trim());}
export class UpgradeDraft {
 constructor(){this.state={path:'',confirmation:null,pending:false,open:false,error:'',notice:''};}
 setPath(value){this.state.path=value;this.state.confirmation=null;this.state.error='';this.state.notice='';}
 confirm(app,accepted){this.state.confirmation=accepted?upgradeSnapshot(app,this.state.path):null;}
 invalidate(){this.state.confirmation=null;}
 ready(app){return !this.state.pending&&matchesUpgrade(this.state.confirmation,app,this.state.path);}
 async run(api,app){if(!this.ready(app))return {ok:false,items:null};const observed=this.state.confirmation;this.state.confirmation=null;this.state.pending=true;this.state.error='';this.state.notice='';let requestError=null,items=null,current=null;
  try{try{await api.post(`/api/apps/${segment(observed.id)}/upgrade`,{path:observed.path,expectedRevision:observed.revision});}catch(error){requestError=error;}
   try{items=await api.list('/api/apps');current=items.find(item=>entityId(item.id)===observed.id)||null;}catch(error){this.state.error=`${requestError?.message||'The upgrade request returned.'} The active registration could not be verified. Do not assume rollback or retry automatically.`;return {ok:false,items:null};}
   const description=current?`Current registration: ${current.version} · revision ${current.revision} · ${current.enabled?'enabled':'disabled'}.`:'The application is absent from the current registration.';
   if(requestError){this.state.error=`Upgrade request did not complete successfully: ${requestError.message||String(requestError)}. ${description} No automatic retry was made.`;return {ok:false,items};}
   if(!current||typeof current.version!=='string'||!current.version||current.version===observed.version||!Number.isSafeInteger(current.revision)||current.revision<=observed.revision||current.enabled!==false){this.state.error=`The host accepted the request, but the expected disabled upgrade state was not verified. ${description}`;return {ok:false,items};}
   this.state.notice=`Upgraded to ${current.version}. The application remains disabled. Review its permissions before explicitly enabling it; old version data is retained.`;return {ok:true,items,current};
  }finally{this.state.pending=false;}
 }
}
