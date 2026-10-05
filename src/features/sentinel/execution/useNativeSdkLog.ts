import {onBeforeUnmount,ref,watch} from "vue";
import {sentinelApi} from "../api";
import {useCommittedRefresh} from "../../../composables/useCommittedRefresh";
import {acceptSdkPage,acceptsSdkHint,emptySdkReplay,type SdkScope} from "./nativeSdkLogContract";

export function useNativeSdkLog(selected:()=>SdkScope|undefined){
 const replay=ref(emptySdkReplay()),loading=ref(false),readFailed=ref(false),connectionUnavailable=ref(false);
 let generation=0,disposed=false;
 const key=()=>{const s=selected();return s?JSON.stringify([s.scanId,s.attempt,s.ownerId??null]):"";};
 watch(key,()=>{++generation;replay.value=emptySdkReplay();loading.value=false;readFailed.value=false;},{flush:"sync"});
 async function refresh(){
  const scope=selected();if(disposed||!scope?.scanId||document.hidden||loading.value)return;
  const request=++generation,view=key(),current=()=>!disposed&&request===generation&&key()===view;
  loading.value=true;
  try{
   for(let pages=0;pages<4&&current()&&!document.hidden;pages++){
    const prior=replay.value;
    const page=await sentinelApi.readNativeSdkLog(scope.scanId,scope.attempt,prior.attempt,prior.cursor,300,scope.ownerId);
    if(!current())return;replay.value=acceptSdkPage(page,scope,prior);readFailed.value=false;
    if(!replay.value.more)break;
   }
  }catch{if(current())readFailed.value=true;}
  finally{if(current()){loading.value=false;if(replay.value.more&&!readFailed.value)committed.catchUp();}}
 }
 const committed=useCommittedRefresh("nest-native-sdk-log",key,p=>acceptsSdkHint(p,selected()),()=>loading.value,refresh,
  unavailable=>{connectionUnavailable.value=unavailable;});
 onBeforeUnmount(()=>{disposed=true;++generation;});
 return {replay,loading,readFailed,connectionUnavailable,refresh};
}
