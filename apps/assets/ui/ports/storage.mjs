import {createAssetPreferenceStore} from '../preferences.mjs';
let store,unsubscribe;
const listeners=new Set();
const disconnected=()=>Object.freeze({state:'unloaded',revision:null,unsaved:false,errorCode:''});
export function assetPreferenceStatus(){return store?.status()??disconnected();}
function publish(){for(const listener of listeners)listener(assetPreferenceStatus());}
export function subscribeAssetPreferences(listener){listeners.add(listener);listener(assetPreferenceStatus());return()=>listeners.delete(listener);}
export function clearAssetPreferences(){unsubscribe?.();unsubscribe=undefined;store?.destroy();store=undefined;publish();}
export async function initializeAssetPreferences(callAction){clearAssetPreferences();const current=createAssetPreferenceStore(callAction);store=current;unsubscribe=current.subscribe(publish);try{await current.reload();}catch{/* The caller displays unavailable; layout stays page-local and no write is admitted. */}return current===store?current.status():disconnected();}
export async function reloadAssetPreferences(){if(!store)throw Error('assets_preferences_not_connected');return store.reload();}
function local(){if(!store)throw Error('assets_preferences_not_connected');return store.storage;}
export const assetLocalStorage=Object.freeze({getItem:key=>local().getItem(key),setItem:(key,value)=>local().setItem(key,value),removeItem:key=>local().removeItem(key)});
// Filter queries remain temporary and are never sent to persistent storage.
const session=new Map();
export const assetSessionStorage=Object.freeze({getItem:key=>session.get(String(key))??null,setItem:(key,value)=>session.set(String(key),String(value)),removeItem:key=>session.delete(String(key))});

export async function saveAssetScanMode(mode){if(!store)throw Error('assets_preferences_not_connected');const observed=store;observed.storage.setItem('asset-agent-scan-mode',mode);const result=await observed.settled();if(store!==observed)throw Object.assign(Error('assets_page_context_changed'),{code:'assets_page_context_changed'});if(result.state!=='ready'||result.unsaved||result.errorCode)throw Object.assign(Error('assets_preferences_handoff_unconfirmed'),{code:'assets_preferences_handoff_unconfirmed'});return result;}
