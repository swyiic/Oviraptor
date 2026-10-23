import {CertificateTasks} from './certificate-tasks.mjs';
export const EXPOSURE_TOOLS = new Map([
 ['exposure_findings','findings'],['exposure_finding','finding'],['exposure_runs','runs'],['exposure_task_status','status'],
 ['exposure_task_start','start'],['exposure_task_resume','resume'],['exposure_task_cancel','cancel'],
]);
export const EXPOSURE_PERMISSIONS=new Map([...EXPOSURE_TOOLS.keys()].map(name=>[name,
 ['exposure_findings','exposure_finding'].includes(name)?'inventory.exposure.details.read':
 ['exposure_runs','exposure_task_status'].includes(name)?'inventory.exposure.read':'inventory.exposure.run']));
export class ExposureTasks extends CertificateTasks {
 constructor(options={}) { super({...options,tools:EXPOSURE_TOOLS,route:'/exposure-task',kind:'exposure',maxResponseBytes:128*1024,
  readOnlyTools:['exposure_findings','exposure_finding','exposure_runs','exposure_task_status']}); }
}
