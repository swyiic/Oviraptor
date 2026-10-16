import test from 'node:test';
import assert from 'node:assert/strict';
import {cloneAssetProfile,editableSettings,profileInput,importCollectionFile} from '../ui/configuration.mjs';
import {reactive} from 'vue';
test('actual Vue reactive profile DTOs can open and save without losing nullable or write-only state',()=>{
 const profile=reactive({id:4,name:'Reactive existing profile',settings:{runProbe:false,modelProfiles:null,fofaKey:'',fofaKeyConfigured:true}});
 const clone=cloneAssetProfile(profile);clone.name='Independent edit draft';assert.equal(profile.name,'Reactive existing profile');assert.equal(clone.settings.modelProfiles,null);
 const draft=editableSettings(profile.settings);assert.equal(draft.settings.runProbe,false);assert.equal(draft.settings.modelProfiles,null);assert.equal(draft.settings.fofaKeyConfigured,undefined);const input=profileInput(profile);assert.equal(input.id,4);assert.equal(input.settings.modelProfiles,null);assert.equal(profile.settings.fofaKeyConfigured,true);
});
test('profile editing preserves every existing known field, explicit null and write-only blank credentials',()=>{
 const source={runProbe:false,modelProfiles:null,fofaKey:'',fofaKeyConfigured:true,collectionConfigurationId:'synthetic-file',nativeAnalyzers:{semgrep:{image:'existing'}}};
 const result=profileInput({id:4,name:'Exact profile',isDefault:true,settings:source});assert.deepEqual(result.settings,{runProbe:false,modelProfiles:null,fofaKey:'',collectionConfigurationId:'synthetic-file',nativeAnalyzers:{semgrep:{image:'existing'}}});assert.equal(source.fofaKeyConfigured,true);assert.equal(result.id,4);
});
test('unrecognized current settings are preserved and saving is refused instead of deleting them',()=>{
 const source={runProbe:false,futurePolicy:{retain:true}};const draft=editableSettings(source);assert.deepEqual(draft.settings,source);assert.deepEqual(draft.unsupported,['futurePolicy']);assert.throws(()=>profileInput({name:'Keep unknown',settings:source}),/unknown_settings_preserved/);
});
test('copying a profile cannot silently reuse a private configuration or default selection',()=>{
 const result=profileInput({id:4,name:'Copy',isDefault:true,settings:{fofaKey:'',configPath:'',collectionConfigurationId:'private-id',runProbe:false}},{copy:true});assert.equal(result.id,undefined);assert.equal(result.isDefault,false);assert.equal(result.settings.collectionConfigurationId,'');assert.equal(result.settings.runProbe,false);
});
test('an explicit replacement provider key clears the prior private-file binding while blank retains it',()=>{
 const profile={id:4,name:'Existing imported profile',settings:{fofaKey:'',configPath:'',collectionConfigurationId:'private-id',runProbe:false}};
 assert.equal(profileInput(profile).settings.collectionConfigurationId,'private-id');
 const changed=profileInput({...profile,settings:{...profile.settings,fofaKey:'SYNTHETIC-NONSECRET-FIXTURE'}});assert.equal(changed.settings.collectionConfigurationId,'');assert.equal(changed.settings.configPath,'');assert.equal(changed.settings.fofaKey,'SYNTHETIC-NONSECRET-FIXTURE');assert.equal(profile.settings.collectionConfigurationId,'private-id');
});
test('configuration import sends bounded explicit file content and accepts only a native UUID receipt',async()=>{
 const id='9ce58d7e-11a2-4dc9-8fe0-4b143fc37749',calls=[];const result=await importCollectionFile({size:4,text:async()=>'data'},async(name,args)=>{calls.push([name,args]);return {configurationId:id,fofaKeyConfigured:true};});assert.equal(result.configurationId,id);assert.deepEqual(calls,[['import_asset_collection_configuration',{content:'data'}]]);
 await assert.rejects(importCollectionFile({size:4,text:async()=>'data'},async()=>({configurationId:'/path',fofaKeyConfigured:true})),/receipt_invalid/);
});
test('oversize bytes and a project switch during File.text are refused before native submission',async()=>{
 let calls=0;const invoke=async()=>calls++;
 await assert.rejects(importCollectionFile({size:65537,text:async()=>''},invoke),/size_invalid/);
 await assert.rejects(importCollectionFile({size:1,text:async()=>'汉'.repeat(23000)},invoke),/size_invalid/);
 await assert.rejects(importCollectionFile({size:1,text:async()=>'x'},invoke,()=>false),/context_changed/);assert.equal(calls,0);
});
