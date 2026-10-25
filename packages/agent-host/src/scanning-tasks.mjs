import { CertificateTasks } from './certificate-tasks.mjs';
import { hostError } from './store.mjs';
// The App SDK's bounded schema subset has no conditional fields. Validate the
// section contract before calling the owner as well as at its Rust boundary.
export function validateScanningResultsArguments(args) {
 const section=args.section??'targets',has=key=>Object.hasOwn(args,key);
 if(args.kind!==undefined&&(typeof args.kind!=='string'||!/^[a-z0-9_]{1,80}$/.test(args.kind))
  ||section==='evidence'&&(!Number.isSafeInteger(args.recordId)||args.recordId<1||has('kind')||has('afterId')||has('limit'))
  ||section==='findings'&&has('recordId')
  ||section==='targets'&&(has('kind')||has('recordId')))throw hostError('host_tool_input_invalid',400);
 return args;
}
export const SCANNING_TOOLS = new Map([
 ['scan_from_assets','from_assets'],['scan_list','list'],['scan_status','status'],['scan_preflight','preflight'],['scan_recovery','recovery'],['scan_results','results'],
 ['scan_start','start'],['scan_pause','pause'],['scan_resume','resume'],
]);
export class ScanningTasks extends CertificateTasks {
 constructor(options={}){super({origin:process.env.OVIRAPTOR_SCAN_BRIDGE_ORIGIN??null,token:process.env.OVIRAPTOR_SCAN_BRIDGE_TOKEN??null,...options,tools:SCANNING_TOOLS,route:'/scan-task',kind:'scan'});}
}
