import { CertificateTasks } from './certificate-tasks.mjs';
export const SCANNING_TOOLS = new Map([
 ['scan_from_assets','from_assets'],['scan_list','list'],['scan_status','status'],['scan_preflight','preflight'],['scan_results','results'],
 ['scan_start','start'],['scan_pause','pause'],['scan_resume','resume'],
]);
export class ScanningTasks extends CertificateTasks {
 constructor(options={}){super({origin:process.env.OVIRAPTOR_SCAN_BRIDGE_ORIGIN??null,token:process.env.OVIRAPTOR_SCAN_BRIDGE_TOKEN??null,...options,tools:SCANNING_TOOLS,route:'/scan-task',kind:'scan'});}
}
