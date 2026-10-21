// Explicit schemas for the old JsonLike payloads. These mirror the named Rust
// business DTOs; a JSON string or unrestricted object is never an escape hatch.
export const string=(maxLength=65536)=>({type:'string',maxLength});
export const integer=()=>({type:'integer',minimum:-Number.MAX_SAFE_INTEGER,maximum:Number.MAX_SAFE_INTEGER});
export const boolean=()=>({type:'boolean'});
export const array=(items,maxItems=16384)=>({type:'array',items,maxItems});
export const object=(properties,required=[])=>({type:'object',properties,required,additionalProperties:false});
export const optionalNull=schema=>({...schema,type:[schema.type,'null']});
const strings=array(string());const ids=array(integer());
export const exposureSettings=object({
 exposureSources:strings,exposureMaxRequests:integer(),exposureMaxResponseBytes:integer(),exposureMaxTotalBytes:integer(),exposureMaxItems:integer(),exposureConcurrency:integer(),exposureTimeoutSeconds:integer(),exposureCtPageSize:integer(),exposureArchivePageSize:integer(),exposureAllowPrivateDestinations:boolean(),exposureAuthorizationDays:integer(),exposureGithubOfficialOwners:strings,exposureGithubCredentialRef:string(),version:integer(),
 exposureDocumentUrls:array(object({url:string(),publicMaterial:boolean(),note:string()},['url'])),
});
export const incidentAction=object({
 operationId:string(80),action:string(80),incidentId:integer(),expectedRevision:integer(),actor:string(80),reason:string(),
 status:optionalNull(string()),targetIncidentId:optionalNull(integer()),targetExpectedRevision:optionalNull(integer()),locationIds:optionalNull(ids),entityId:optionalNull(integer()),linkLevel:optionalNull(string()),evidenceObservationIds:optionalNull(ids),handling:optionalNull(string()),channel:optionalNull(string()),externalReference:optionalNull(string()),retestRunId:optionalNull(integer()),
},['operationId','action','incidentId','expectedRevision','actor','reason']);
export const importRequest=object({
 operationId:string(80),channel:string(80),operator:string(80),sourceDeclaration:string(),declaredUrl:string(),batchLabel:string(),publicMaterial:boolean(),publicMaterialNote:string(),extractedFrom:string(),
 files:array(object({name:string(512),format:string(80),contentBase64:string(1024*1024)},['name','format','contentBase64']),20),
},['operationId','channel','operator','sourceDeclaration','files']);
export const gradeRequest=object({operationId:string(80),findingId:integer(),expectedGrade:string(80),grade:string(80),actor:string(80),reason:string(),sampleObservationId:optionalNull(integer()),entityId:optionalNull(integer())},['operationId','findingId','expectedGrade','grade','actor','reason']);
export const reclassificationRequest=object({operationId:string(80),findingId:integer(),expectedClassificationId:integer(),disposition:string(80),infoCategory:string(80),actor:string(80),reason:string()},['operationId','findingId','expectedClassificationId','disposition','infoCategory','actor','reason']);
export const assetFilter=object({query:string(120),decisionView:string(80),probeView:string(80),probeOutcomeView:string(80),deletedView:string(80)});
export const manualOverrides={save_exposure_settings:{settings:exposureSettings},act_on_exposure_incident:{action:incidentAction},import_exposure_documents:{request:importRequest},grade_exposure_intel:{request:gradeRequest},reclassify_exposure_finding:{request:reclassificationRequest}};
