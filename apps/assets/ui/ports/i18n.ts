import {computed,ref} from 'vue';
export type Locale='zh'|'en';
const locale=ref<Locale>('zh');
export function useI18n(){const tr=(zh:string,en:string)=>locale.value==='en'?en:zh;return {locale,tr,setLocale:(value:Locale)=>{if(value==='en'||value==='zh')locale.value=value;},t:computed(()=>({assets:tr('资产','Assets'),query:tr('采集','Collection'),tasks:tr('任务','Tasks'),logs:tr('日志','Logs'),changes:tr('变化','Changes'),refresh:tr('刷新','Refresh'),empty:tr('暂无记录','No records')}))};}
