import {ASSET_ACTIONS} from '../operations.mjs';
const actions=new Set(ASSET_ACTIONS);
let client,context,revision=0;
const failure=code=>Object.assign(Error(code),{code});
export function bindAssetsClient(value,nativeContext){if(!value||typeof value.callAction!=='function'||!Number.isSafeInteger(value.context?.projectId)||!nativeContext||nativeContext.hostProjectId!==value.context.projectId||!Number.isSafeInteger(nativeContext.nativeProjectId)||nativeContext.nativeProjectId<1||typeof nativeContext.name!=='string'||typeof nativeContext.status!=='string')throw failure('assets_context_invalid');client=value;context=Object.freeze({...nativeContext});revision++;}
export function clearAssetsClient(){client=undefined;context=undefined;revision++;}
export function assetsContext(){return context;}
export function assetsContextRevision(){return revision;}
function clean(value){if(Array.isArray(value))return value.map(item=>{if(item===undefined)throw failure('assets_arguments_invalid');return clean(item);});if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).filter(([,item])=>item!==undefined).map(([key,item])=>[key,clean(item)]));return value;}
function checkScope(args){const id=context.nativeProjectId;for(const value of [args,args.input,args.query,args.request?.query,args.input?.rule])for(const key of ['projectId','expectedProjectId'])if(value?.[key]!==undefined&&value[key]!==id)throw failure('assets_project_scope_changed');for(const value of [args.selections,args.input?.selections])if(Array.isArray(value)&&value.some(item=>item?.projectId!==id))throw failure('assets_project_scope_changed');}
export async function invoke(command,args={}){if(!client||!context)throw failure('assets_page_not_connected');if(!actions.has(command))throw failure('assets_action_not_declared');const parameters=clean(args);checkScope(parameters);const observed=revision,value=await client.callAction(command,parameters);if(observed!==revision)throw failure('assets_page_context_changed');return value;}
export async function openAssetUrl(){throw failure('assets_external_open_not_connected');}
