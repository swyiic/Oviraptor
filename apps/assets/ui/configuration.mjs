import schemas from '../native-action-schemas.json' with {type:'json'};
import {toRaw} from 'vue';
const known=new Set(schemas.profileSettingsFields);
export function cloneAssetProfile(source){return structuredClone(toRaw(source));}
export function editableSettings(source){
 if(!source||typeof source!=='object'||Array.isArray(source))throw Error('assets_profile_settings_invalid');
 const settings=cloneAssetProfile(source);
 function removeReadFlags(value){if(Array.isArray(value)){value.forEach(removeReadFlags);return;}if(value&&typeof value==='object')for(const key of Object.keys(value)){if(key.endsWith('Configured'))delete value[key];else removeReadFlags(value[key]);}}
 removeReadFlags(settings);
 const unsupported=Object.keys(settings).filter(key=>!known.has(key));
 return {settings,unsupported};
}
export function profileInput(profile,{copy=false}={}){
 const {settings,unsupported}=editableSettings(profile.settings);if(unsupported.length)throw Error('assets_profile_unknown_settings_preserved');
 const input={name:profile.name,description:profile.description??'',isDefault:copy?false:profile.isDefault===true,settings};
 if(!copy&&Number.isSafeInteger(profile.id))input.id=profile.id;
 // The native service intentionally gives an explicit private file binding
 // precedence. A newly typed key means the operator is replacing that source.
 if(typeof settings.fofaKey==='string'&&settings.fofaKey.trim()){
  settings.collectionConfigurationId='';settings.configPath='';
 }
 // Copies never inherit a hidden credential or private file implicitly.
 if(copy){settings.collectionConfigurationId='';settings.configPath='';}
 return input;
}
export async function importCollectionFile(file,invoke,isCurrent=()=>true){
 if(!file||!Number.isSafeInteger(file.size)||file.size<1||file.size>65536)throw Error('assets_collection_configuration_size_invalid');
 let content=await file.text();if(!isCurrent())throw Error('assets_page_context_changed');
 if(new TextEncoder().encode(content).length>65536)throw Error('assets_collection_configuration_size_invalid');
 try{const receipt=await invoke('import_asset_collection_configuration',{content});if(!isCurrent())throw Error('assets_page_context_changed');if(typeof receipt?.configurationId!=='string'||!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(receipt.configurationId)||receipt.fofaKeyConfigured!==true)throw Error('assets_collection_configuration_receipt_invalid');return receipt;}finally{content='';}
}
