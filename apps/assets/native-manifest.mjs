import schemas from './native-action-schemas.json' with {type:'json'};
import owners from './contracts/native-action-owners.json' with {type:'json'};
import {ARTIFACT_READ_SCHEMA} from '../../packages/app-sdk/src/index.mjs';
const descriptions={
 'inventory.read':'Read this project’s saved Assets inventory, rules, ownership and exposure evidence. Saved observations do not prove current liveness or external coverage.',
 'inventory.write':'Explicitly import targets, review validity or ownership facts, archive or restore assets, and run local ownership analysis in this project. Ownership facts do not grant execution authorization.',
 'inventory.rules.manage':'Previewed project page isolation and restoration. Other manual decisions and protected rows remain subject to the original rules.',
 'inventory.rules.global':'Application-wide content rule administration. Changes can affect every project in this Assets application; operator actions only.',
 'inventory.export':'Create project-owned CSV exports and retrieve only issued, instance-bound artifact receipts. No arbitrary file paths.',
 'inventory.jobs.run':'Explicitly start, resume or cancel the original project collection/probe pipeline. May contact configured providers and targets; accepted requests are not completion.',
 'inventory.configuration.manage':'Application-wide execution profile and write-only credential administration, including private configuration file content import. No implicit provider test; operator actions only.',
 'inventory.ownership.directory.manage':'Application-wide organization facts and directory bindings. May affect all projects in this Assets application; does not itself grant execution authorization.',
 'inventory.authorization.manage':'Explicitly grant or revoke project asset execution authorization under the original current-row and snapshot CAS gates. Separate from ownership confirmation.',
 'inventory.exposure.run':'Explicitly start, resume, retest or cancel project exposure collection under the original authorization, budget and source gates. May contact external sources and targets.',
 'inventory.exposure.manage':'Manually manage saved exposure findings, classifications, incidents, remediation and project collection settings; no automatic external execution.',
};
export function createNativeManifest({platform=process.platform,arch=process.arch}={}){
 const actions=owners.actions.map(item=>{
  const inputSchema=schemas.schemas[item.name];if(!inputSchema)throw Error('assets_native_schema_missing:'+item.name);
  const description=item.name==='read_asset_artifact'?'Read one bounded chunk from a current-project issued export receipt, rechecking the native instance, file identity and SHA on every request.':`${item.name}: ${descriptions[item.permission]}`;
  return {name:item.name,scope:item.scope,permission:item.permission,description,inputSchema:item.name==='read_asset_artifact'?ARTIFACT_READ_SCHEMA:inputSchema,...(item.name==='read_asset_artifact'?{delivery:'artifact-read'}:{})};
 });
 const tools=owners.actions.filter(item=>item.modelTool).map(item=>{
  const schema=structuredClone(schemas.schemas[item.name]);
  const description=item.name==='saved_assets_stats'?'Exact saved-member statistics in the current project under the supplied explicit filter. Includes unknown saved statuses; not a statement of current reachability or complete provider discovery.':'Read at most 20 saved project asset members with an exact filtered total and scoped cursor. Literal metadata search; no collection, probe, authorization or implicit external call.';
  return{name:item.name,permission:item.permission,description,inputSchema:schema};
 });
 return{schemaVersion:2,id:'org.oviraptor.assets',version:'2.0.0',hostApi:'^2.0.0',runtime:{kind:'native',platform,arch},entry:`bin/oviraptor-assets-service${platform==='win32'?'.exe':''}`,dataSchemaVersion:1,permissions:Object.entries(descriptions).map(([id,description])=>({id,description})),tools,actions,pages:[{id:'home',title:'Assets',path:'page.html'}],lifecycle:{start:true,stop:true}};
}
