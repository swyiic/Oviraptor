<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  Activity, Bell, BrainCircuit, Braces, Bug, CheckCircle2, ChevronDown, CircleAlert, ClipboardCheck, Clock3, Database,
  AppWindow, FileClock, FolderKanban, Globe2, Languages, LayoutDashboard, ListFilter, Menu, Palette, PlayCircle, Plus, RefreshCw, Search,
  Settings2, ShieldAlert, ShieldCheck, Server, SlidersHorizontal, TerminalSquare, Trash2, X,
} from "@lucide/vue";
import { api } from "./api";
import packageInfo from "../package.json";
import { useI18n } from "./i18n";
import type { AppSettings, AssetEvent, ConfigProfile, DashboardStats, EnvironmentReport, HackerOneEvent, InterruptedJob, JobProgressEvent, JobRun, LogEntry, Project, ProjectImpact, SentinelScan, StartupStatus, StrixUpdateStatus, ToastMessage, ViewKey } from "./types";
import AssetWorkspace from "./components/AssetWorkspace.vue";
import AppSettingsDialog from "./components/AppSettingsDialog.vue";
import ConfigDialog from "./components/ConfigDialog.vue";
import HackerOneBoard from "./components/HackerOneBoard.vue";
import SentinelBoard from "./components/SentinelBoard.vue";
import ProjectDialog from "./components/ProjectDialog.vue";
import InlineConfirm from "./components/InlineConfirm.vue";
import QueryPanel from "./components/QueryPanel.vue";
import ReleaseNotesDialog from "./components/ReleaseNotesDialog.vue";
import WorkerSettingsPanel from "./components/WorkerSettingsPanel.vue";
import defaultBrandIconUrl from "../src-tauri/icons/brand-icon.png";
import "./styles.css";
import "./probe-status.css";
import "./theme-enhancements.css";

const { locale, t, tr, setLocale } = useI18n();
type ModuleKey = "hackerone" | "asset" | "sentinel";
for(const suffix of ["view","module","project","theme","accent"]){
  const current=`oviraptor-${suffix}`;const legacy=`asset-atlas-${suffix}`;
  if(localStorage.getItem(current)===null&&localStorage.getItem(legacy)!==null)localStorage.setItem(current,localStorage.getItem(legacy)!);
  localStorage.removeItem(legacy);
}
const activeView = ref<ViewKey>((localStorage.getItem("oviraptor-view") as ViewKey) || "dashboard");
const activeModule = ref<ModuleKey>((localStorage.getItem("oviraptor-module") as ModuleKey) || (activeView.value === "hackerone" ? "hackerone" : activeView.value === "sentinel" ? "sentinel" : "asset"));
const projects = ref<Project[]>([]); const profiles = ref<ConfigProfile[]>([]); const runs = ref<JobRun[]>([]);
const logs = ref<LogEntry[]>([]); const events = ref<AssetEvent[]>([]); const stats = ref<DashboardStats>();
const appSettings = ref<AppSettings>({ reminderDays: 7, customIcon: false, deduplicatedAssets: 0 });
const startup = ref<StartupStatus>({ reminderDays: 7, staleProjects: [], interruptedJobs: [] });
const selectedProjectId = ref<number | undefined>(Number(localStorage.getItem("oviraptor-project")) || undefined);
const projectDialog = ref(false); const editProject = ref<Project>(); const configDialog = ref(false); const editProfile = ref<ConfigProfile>();
const appSettingsDialog = ref(false); const startupNotice = ref(true);
const releaseNotesDialog = ref(false);
const h1HasChanges = ref(false);
const environment = ref<EnvironmentReport>();
const environmentChecking = ref(false);
const environmentInstalling = ref(false);
const strixUpdate = ref<StrixUpdateStatus>();
const strixUpdateChecking = ref(false);
const strixUpdating = ref(false);
const strixUpdateDismissed = ref(false);
type EnvironmentInstallLog = { stage: string; stream: string; message: string; time: string };
const environmentInstallLogs = ref<EnvironmentInstallLog[]>([]);
const environmentInstallState = ref<"idle" | "running" | "success" | "error">("idle");
const environmentInstallError = ref("");
const environmentInstallConsole = ref<HTMLElement>();
const configSection = ref<"profiles" | "workers" | "runtime">("profiles");
const deletingProfileId = ref<number>();
const sidebarCollapsed = ref(false); const globalSearch = ref(""); const loading = ref(true); const toasts = ref<ToastMessage[]>([]);
const deletingProjectId = ref<number>();
const pendingProjectAction = ref<{ project: Project; impact: ProjectImpact; mode: "delete" | "archive" }>();
const storedTheme=localStorage.getItem("oviraptor-theme")||"cloud";
const themePreset=ref(storedTheme==="cloud"?"cloud":"codex");
if(storedTheme!==themePreset.value)localStorage.setItem("oviraptor-theme",themePreset.value);
const accentColor=ref(localStorage.getItem("oviraptor-accent")||"#2878ff");
const showTheme=ref(false);
const sentinelSearch = ref("");
const sentinelSection = ref<"overview"|"queue"|"results"|"fuse"|"validations"|"workbench"|"help">("overview");
const sentinelResultView = ref<"summary"|"fingerprint"|"api"|"endpoints"|"vulnerabilities">("summary");
const sentinelWorkbenchMode = ref<"web"|"code"|"greybox"|"cicd"|"skills"|"traces">("code");
const sentinelMenu = ref("overview");
const sentinelLogScans=ref<SentinelScan[]>([]); const hackerOneLogEvents=ref<HackerOneEvent[]>([]);
const sentinelLogScanId=ref(""); const sentinelRunnerLogs=ref<string[]>([]); const sentinelRunnerLogLoading=ref(false);
const sentinelRunnerConsole=ref<HTMLElement>();
const sentinelAlerts=ref({fuse:0,vulnerabilities:0});
const brandIconUrl=ref(defaultBrandIconUrl);
let unlisten: UnlistenFn | undefined;
let unlistenTray: UnlistenFn | undefined;
let unlistenEnvironmentInstall: UnlistenFn | undefined;
let workerSyncTimer: ReturnType<typeof setInterval> | undefined;
let sentinelLogTimer: ReturnType<typeof setInterval> | undefined;
let strixUpdateTimer: ReturnType<typeof setTimeout> | undefined;
let sentinelRunnerLogRequest=false;
let workerAutoSyncRunning=false;

const selectedProject = computed(() => projects.value.find(project => project.id === selectedProjectId.value));
const activeProfile = computed(() => profiles.value.find(profile=>profile.isDefault)||profiles.value[0]);
const hackerOneEnabled = computed(() => Boolean(activeProfile.value?.settings?.hackerOneUsername?.trim() && activeProfile.value?.settings?.hackerOneToken?.trim()));
const viewTitle = computed(() => {
  if (activeView.value === "sentinel") {
    const titles: Record<string, string> = {
      overview: tr("行动中心", "Investigation desk"),
      queue: tr("任务与成本", "Tasks & cost"),
      strix_scan: tr("自动调查", "Automated investigation"),
      urls: tr("证据中心", "Evidence center"),
      fuse: tr("停止与熔断", "Stop & fuse"),
      vulnerabilities: tr("漏洞结论", "Vulnerabilities"),
      validations: tr("验证记录", "Verification log"),
      traces: tr("运行轨迹", "Execution traces"),
      skills: tr("知识与策略", "Knowledge & policy"),
    };
    return titles[sentinelMenu.value] || titles.overview;
  }
  return ({
    dashboard:t.value.dashboard, projects:t.value.projects, query:t.value.query, assets:t.value.assets,
    quarantine:t.value.quarantine, hackerone:"HackerOne SRC", sentinel:"安全总览", changes:t.value.changes, tasks:t.value.tasks, logs:t.value.logs, settings:t.value.settings,
  })[activeView.value];
});
const nav = computed(() => [
  { key:"dashboard" as ViewKey,label:t.value.dashboard,icon:LayoutDashboard },
  { key:"projects" as ViewKey,label:t.value.projects,icon:FolderKanban },
  { key:"query" as ViewKey,label:t.value.query,icon:Search },
  { key:"assets" as ViewKey,label:t.value.assets,icon:Database },
  { key:"quarantine" as ViewKey,label:t.value.quarantine,icon:ShieldAlert,badge:stats.value?.blockedCount },
  { key:"changes" as ViewKey,label:t.value.changes,icon:FileClock },
  { key:"tasks" as ViewKey,label:t.value.tasks,icon:Activity,badge:stats.value?.runningJobs },
].filter(() => activeModule.value === "asset"));
const sentinelNav = computed(()=>[
  {key:"overview",section:"overview" as const,label:tr("行动中心","Investigation desk"),icon:Activity},
  {key:"strix_scan",section:"workbench" as const,workbench:"web" as const,label:tr("自动调查","Automated investigation"),icon:Globe2},
  {key:"queue",section:"queue" as const,label:tr("任务与成本","Tasks & cost"),icon:ClipboardCheck},
  {key:"urls",section:"results" as const,result:"summary" as const,label:tr("证据中心","Evidence center"),icon:Globe2},
  {key:"vulnerabilities",section:"results" as const,result:"vulnerabilities" as const,label:tr("漏洞结论","Vulnerabilities"),icon:Bug,badge:sentinelAlerts.value.vulnerabilities},
  {key:"validations",section:"validations" as const,label:tr("验证记录","Verification log"),icon:ShieldCheck},
  {key:"fuse",section:"fuse" as const,label:tr("停止与熔断","Stop & fuse"),icon:ShieldAlert,badge:sentinelAlerts.value.fuse},
  {key:"traces",section:"workbench" as const,workbench:"traces" as const,label:tr("运行轨迹","Execution traces"),icon:BrainCircuit},
  {key:"skills",section:"workbench" as const,workbench:"skills" as const,label:tr("知识与策略","Knowledge & policy"),icon:Braces},
]);

function navigate(view: ViewKey) { activeView.value=view; if(view==='hackerone') activeModule.value='hackerone'; if(view==='sentinel')activeModule.value='sentinel'; localStorage.setItem("oviraptor-view",view); localStorage.setItem("oviraptor-module",activeModule.value); window.setTimeout(()=>void refreshSecondary(),0); }
function navigateFromTray(destination: string) {
  if (destination === "assets") {
    switchModule("asset");
    navigate("assets");
  } else if (destination === "strix-tasks") {
    switchModule("sentinel");
    sentinelMenu.value = "queue";
    sentinelSection.value = "queue";
    navigate("sentinel");
  }
}
function switchModule(module: ModuleKey) { if(module==="hackerone"&&!hackerOneEnabled.value){notify("info",tr("请先在配置中心填写 HackerOne API identifier 和 token","Configure HackerOne API credentials first"));return} activeModule.value=module; localStorage.setItem("oviraptor-module",module); navigate(module === "hackerone" ? "hackerone" : module === "sentinel" ? "sentinel" : "dashboard"); }
function openSentinelSection(item:(typeof sentinelNav.value)[number]){sentinelMenu.value=item.key;sentinelSection.value=item.section;if("result" in item&&item.result)sentinelResultView.value=item.result;if("workbench" in item&&item.workbench)sentinelWorkbenchMode.value=item.workbench;navigate("sentinel")}
function selectProject(id?: number) { selectedProjectId.value=id; id ? localStorage.setItem("oviraptor-project",String(id)) : localStorage.removeItem("oviraptor-project"); void refreshProjectData(); }
function notify(type: "success"|"error"|"info", text: string) { const item={id:Date.now()+Math.random(),type,text}; toasts.value.push(item); setTimeout(()=>toasts.value=toasts.value.filter(t=>t.id!==item.id),5000); }
function sentinelErrorDetail(scan: SentinelScan) {
  const checkpoint = scan.currentCheckpoint || "";
  const marker = "报错细节：";
  const markerIndex = checkpoint.indexOf(marker);
  if (markerIndex >= 0) return checkpoint.slice(markerIndex + marker.length).trim();
  return scan.status === "failed" ? checkpoint : "";
}
function sentinelCheckpointSummary(scan: SentinelScan) {
  const checkpoint = scan.currentCheckpoint || tr("等待任务状态", "Waiting for status");
  const markerIndex = checkpoint.indexOf("；报错细节：");
  return markerIndex >= 0 ? checkpoint.slice(0, markerIndex) : checkpoint;
}
function selectSentinelLogScan(scanId:string){ sentinelLogScanId.value=scanId; void refreshSentinelRunnerLog(); }
function syncSentinelLogSelection(){
  const selected=sentinelLogScans.value.find(scan=>scan.id===sentinelLogScanId.value);
  if(selected)return;
  const preferred=sentinelLogScans.value.find(scan=>["scanning","pausing","queued"].includes(scan.status))||sentinelLogScans.value[0];
  sentinelLogScanId.value=preferred?.id||"";
}
async function refreshSentinelRunnerLog(){
  if(!sentinelLogScanId.value||sentinelRunnerLogRequest)return;
  sentinelRunnerLogRequest=true; sentinelRunnerLogLoading.value=true;
  try{ sentinelRunnerLogs.value=await api.getSentinelRunnerLog(sentinelLogScanId.value,300); }
  catch(error){ sentinelRunnerLogs.value=[`[runner log unavailable] ${String(error)}`]; }
  finally{
    sentinelRunnerLogRequest=false; sentinelRunnerLogLoading.value=false;
    await nextTick();
    const output=sentinelRunnerConsole.value;
    if(output) output.scrollTop=output.scrollHeight;
  }
}
async function pollSentinelActivity(){
  if(activeView.value!=="logs"||activeModule.value!=="sentinel")return;
  try{
    sentinelLogScans.value=await api.listSentinelScans();
    syncSentinelLogSelection();
    await refreshSentinelRunnerLog();
  }catch{/* The next polling tick will retry without interrupting the UI. */}
}
function setTheme(preset:string){themePreset.value=preset;localStorage.setItem("oviraptor-theme",preset);showTheme.value=false}
function saveAccent(){localStorage.setItem("oviraptor-accent",accentColor.value)}
let refreshGeneration=0;
async function refreshSecondary(generation=refreshGeneration) {
  const view=activeView.value;
  const module=activeModule.value;
  const projectId=selectedProjectId.value;
  const requests:Promise<void>[]=[];
  if(module==="asset"&&["dashboard","tasks"].includes(view)) requests.push(api.listRuns(projectId,100).then(value=>{if(generation===refreshGeneration)runs.value=value;}));
  if(module==="asset"&&["dashboard","changes"].includes(view)) requests.push(api.listEvents(projectId,undefined,200).then(value=>{if(generation===refreshGeneration)events.value=value;}));
  if(view==="logs"&&module==="asset") requests.push(api.listLogs(undefined,300,projectId).then(value=>{if(generation===refreshGeneration)logs.value=value;}));
  if(view==="logs"&&module==="sentinel") requests.push(api.listSentinelScans(projectId,300).then(value=>{if(generation===refreshGeneration){sentinelLogScans.value=value;syncSentinelLogSelection();void refreshSentinelRunnerLog();}}));
  if(view==="logs"&&module==="hackerone"&&hackerOneEnabled.value) requests.push(api.listHackerOneEvents(300).then(value=>{if(generation===refreshGeneration)hackerOneLogEvents.value=value;}));
  await Promise.allSettled(requests);
}
async function refreshProjectData(){
  const generation=++refreshGeneration;
  try{stats.value=await api.dashboardStats(selectedProjectId.value);await refreshSecondary(generation);}
  catch(error){notify("error",String(error));}
}
async function refresh(waitForSecondary: boolean | Event=false) {
  const shouldWait=waitForSecondary===true;
  const generation=++refreshGeneration;
  try {
    [projects.value,profiles.value,stats.value,appSettings.value] = await Promise.all([
      api.listProjects(),api.listProfiles(),api.dashboardStats(selectedProjectId.value),api.getAppSettings(),
    ]);
    const secondary=refreshSecondary(generation);
    if(shouldWait)await secondary;
    else void secondary;
    if(hackerOneEnabled.value)void api.listHackerOneEvents(1).then(value=>h1HasChanges.value=value.length>0);
    if(activeModule.value==="hackerone"&&!hackerOneEnabled.value){activeModule.value="asset";activeView.value="dashboard";localStorage.setItem("oviraptor-module","asset");localStorage.setItem("oviraptor-view","dashboard")}
    if (selectedProjectId.value && !projects.value.some(p=>p.id===selectedProjectId.value)){selectedProjectId.value=undefined;localStorage.removeItem("oviraptor-project");}
  } catch(error){ notify("error",String(error)); }
  finally { loading.value=false; }
}
async function projectSaved(id:number){ projectDialog.value=false;editProject.value=undefined;await refresh();selectProject(id); }
async function profileSaved(){ configDialog.value=false;editProfile.value=undefined;await refresh(); }
function openProject(project?:Project){ editProject.value=project;projectDialog.value=true; }
function openProjectWorkspace(project: Project, module: "asset" | "sentinel") {
  selectProject(project.id);
  if (module === "sentinel") {
    switchModule("sentinel");
    sentinelMenu.value = "overview";
    sentinelSection.value = "overview";
  } else {
    switchModule("asset");
  }
}
function projectActivity(project: Project) {
  return [project.lastRunAt, project.lastScanAt].filter(Boolean).sort().reverse()[0] || "—";
}
function projectImpactText(impact: ProjectImpact) {
  const parts = [
    impact.assetCount ? `${impact.assetCount} 条资产` : "",
    impact.assetRunCount ? `${impact.assetRunCount} 次资产任务` : "",
    impact.assetEventCount ? `${impact.assetEventCount} 条资产历史` : "",
    impact.targetCount ? `${impact.targetCount} 个采集目标` : "",
    impact.sentinelScanCount ? `${impact.sentinelScanCount} 个 Strix 任务` : "",
    impact.findingCount ? `${impact.findingCount} 条证据` : "",
    impact.validationCount ? `${impact.validationCount} 条验证` : "",
    impact.appsecVulnerabilityCount ? `${impact.appsecVulnerabilityCount} 条漏洞结论` : "",
    impact.fuseCount ? `${impact.fuseCount} 条停止记录` : "",
    impact.knowledgeCount ? `${impact.knowledgeCount} 条知识` : "",
    impact.savedViewCount ? `${impact.savedViewCount} 个保存视图` : "",
    impact.learningCandidateCount ? `${impact.learningCandidateCount} 个学习候选` : "",
    impact.browserAuthSessionCount ? `${impact.browserAuthSessionCount} 个浏览器会话` : "",
  ].filter(Boolean);
  return parts.join("、") || tr("没有关联数据", "No linked data");
}
function openProfile(profile?:ConfigProfile){ editProfile.value=profile;configDialog.value=true; }
function cloneDefaultProfile(){
  const source=profiles.value.find(item=>item.isDefault)||profiles.value[0];
  if(!source){notify("error",tr("没有可复制的系统配置","No system profile is available to copy"));return;}
  editProfile.value={...source,id:0,name:`${source.name} ${tr("副本","Copy")}`,description:tr("从系统默认配置创建，可独立调整。","Created from the system default and editable independently."),isDefault:false,settings:structuredClone(source.settings),createdAt:"",updatedAt:""};
  configDialog.value=true;
}
async function removeProfile(profile:ConfigProfile){
  if(profile.isDefault){notify("info",tr("系统默认配置不能删除","The system default profile cannot be deleted"));return;}
  if(deletingProfileId.value!==profile.id){deletingProfileId.value=profile.id;notify("info",tr("再次点击删除以确认","Click Delete again to confirm"));return;}
  try{await api.deleteProfile(profile.id);await refresh();notify("success",tr("配置已删除","Profile deleted"));}
  catch(error){notify("error",String(error));}
  finally{deletingProfileId.value=undefined;}
}
async function loadBrandIcon(){
  if(!appSettings.value.customIcon){brandIconUrl.value=defaultBrandIconUrl;return;}
  try{brandIconUrl.value=await api.getAppIconDataUrl();}
  catch{brandIconUrl.value=defaultBrandIconUrl;}
}
async function appSettingsSaved(){ appSettingsDialog.value=false; await refresh(); await loadBrandIcon(); }
function doGlobalSearch(){ if(activeModule.value==="sentinel"){sentinelSection.value="results";navigate("sentinel");return} if(!globalSearch.value.trim())return; navigate("assets"); }
async function checkEnvironment(){
  environmentChecking.value=true;
  try {
    environment.value=await api.checkEnvironment(profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id);
    notify("success",tr("环境检查完成","Environment check completed"));
  } catch(error){ notify("error",String(error)); }
  finally { environmentChecking.value=false; }
}
async function installEnvironment(){
  environmentInstalling.value=true;
  environmentInstallLogs.value=[];
  environmentInstallError.value="";
  environmentInstallState.value="running";
  try {
    const profileId=profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id;
    const message=await api.installEnvironmentDependencies(profileId);
    environment.value=await api.checkEnvironment(profileId);
    environmentInstallState.value="success";
    notify("success",message);
  } catch(error){
    const message=String(error);
    environmentInstallError.value=message;
    environmentInstallState.value="error";
    if(!environmentInstallLogs.value.some(item=>item.message===message)){
      environmentInstallLogs.value.push({stage:"failed",stream:"error",message,time:new Date().toLocaleTimeString()});
    }
    notify("error",message);
  } finally { environmentInstalling.value=false; }
}
async function checkStrixUpdate(force=false, announce=false){
  if(strixUpdateChecking.value||strixUpdating.value)return;
  strixUpdateChecking.value=true;
  try{
    const profileId=profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id;
    strixUpdate.value=await api.checkStrixUpdate(profileId,force);
    if(announce){
      if(strixUpdate.value.checkError)notify("error",strixUpdate.value.checkError);
      else if(strixUpdate.value.updateAvailable)notify("info",tr(`Strix ${strixUpdate.value.latestVersion} 可用`,`Strix ${strixUpdate.value.latestVersion} is available`));
      else notify("success",tr("Strix 已是最新版本","Strix is up to date"));
    }
  }catch(error){if(announce)notify("error",String(error));}
  finally{strixUpdateChecking.value=false;}
}
function openStrixUpdate(){
  activeModule.value="asset";
  localStorage.setItem("oviraptor-module","asset");
  configSection.value="runtime";
  navigate("settings");
}
async function updateStrix(){
  if(strixUpdating.value||environmentInstalling.value)return;
  strixUpdating.value=true;
  environmentInstalling.value=true;
  environmentInstallLogs.value=[];
  environmentInstallError.value="";
  environmentInstallState.value="running";
  try{
    const profileId=profiles.value.find(item=>item.isDefault)?.id || profiles.value[0]?.id;
    strixUpdate.value=await api.updateStrix(profileId);
    environment.value=await api.checkEnvironment(profileId);
    environmentInstallState.value="success";
    strixUpdateDismissed.value=true;
    notify("success",tr(`Strix 已升级到 ${strixUpdate.value.currentVersion}`,`Strix updated to ${strixUpdate.value.currentVersion}`));
  }catch(error){
    const message=String(error);
    environmentInstallError.value=message;
    environmentInstallState.value="error";
    if(!environmentInstallLogs.value.some(item=>item.message===message))environmentInstallLogs.value.push({stage:"failed",stream:"error",message,time:new Date().toLocaleTimeString()});
    notify("error",message);
  }finally{
    strixUpdating.value=false;
    environmentInstalling.value=false;
  }
}
function stageLabel(stage:string){ const zh=({queued:"等待",collect:"采集",import:"入库",refine:"分层",probe:"探测",reprobe:"存活复测",completed:"完成",failed:"失败",cancelled:"取消",interrupted:"已中断",restarted:"已继续"} as any)[stage]; const en=({queued:"Queued",collect:"Collect",import:"Import",refine:"Refine",probe:"Probe",reprobe:"Re-probe",completed:"Completed",failed:"Failed",cancelled:"Cancelled",interrupted:"Interrupted",restarted:"Restarted"} as any)[stage]; return tr(zh||stage,en||stage); }
function statusTone(status:string){ return ["completed"].includes(status)?"success":["failed","interrupted"].includes(status)?"danger":["running","queued","cancel_requested"].includes(status)?"running":"muted"; }
function eventLabel(type:string){ const zh=({new:"新增",changed:"变化",missing:"未发现",decision:"人工结论",archived:"归档",restored:"恢复"} as any)[type]; const en=({new:"New",changed:"Changed",missing:"Not seen",decision:"Decision",archived:"Archived",restored:"Restored"} as any)[type]; return tr(zh||type,en||type); }

async function quickStartProject(projectId:number, profileId?:number, name?:string, pipeline="full") {
  const running=runs.value.find(run=>run.projectId===projectId && ["running","queued","cancel_requested"].includes(run.status));
  if(running){ notify("info",tr(`项目已有运行中的任务 #${running.id}`,`Project already has running job #${running.id}`)); navigate("tasks"); return; }
  const profile=profiles.value.find(item=>item.id===profileId) || profiles.value.find(item=>item.isDefault) || profiles.value[0];
  const project=projects.value.find(item=>item.id===projectId);
  if(!project){ notify("error",tr("工作空间不存在或已被删除","Workspace no longer exists")); return; }
  if(project.status==="archived"){
    notify("info",tr("该工作空间已归档；请先在“项目与范围”中恢复后再创建或复测任务","This workspace is archived. Restore it in Projects and scope before creating or re-probing tasks"));
    return;
  }
  if(!profile){ notify("error",tr("请先创建配置方案","Create a configuration profile first")); return; }
  try {
    const runId=await api.startJob(projectId,profile.id,name||tr(`${project?.name||'项目'} 增量更新`,`${project?.name||'Project'} incremental update`),pipeline);
    notify("success",tr(`任务 #${runId} 已启动，请保持电脑和应用运行`,`Job #${runId} started; keep the computer and app running`));
    navigate("tasks"); await refresh(true);
  } catch(error){ notify("error",String(error)); }
}

async function resumeInterrupted(job:InterruptedJob){
  try { if(job.pipeline==="reprobe"){await api.resumeJob(job.runId);notify("success",tr(`任务 #${job.runId} 已从断点继续`,`Job #${job.runId} resumed from checkpoint`));navigate("tasks");await refresh();}else{await api.acknowledgeInterruptedRun(job.runId);await quickStartProject(job.projectId,job.profileId,`${job.name} · ${tr('继续','Resume')}`,job.pipeline||"full");} startup.value.interruptedJobs=startup.value.interruptedJobs.filter(item=>item.runId!==job.runId); }
  catch(error){ notify("error",String(error)); }
}

async function removeProject(project:Project){
  if(deletingProjectId.value!==undefined)return;
  try {
    const impact=await api.projectImpact(project.id);
    if(impact.totalRecords>0&&project.status==="archived"){
      notify("info",tr(`该工作空间仍有关联数据：${projectImpactText(impact)}。已归档数据不能物理删除。`,`This archived workspace still has linked data: ${projectImpactText(impact)}. It cannot be physically deleted.`));
      return;
    }
    pendingProjectAction.value={project,impact,mode:impact.totalRecords>0?"archive":"delete"};
  }
  catch(error){ notify("error",String(error)); }
}

async function confirmProjectAction(){
  const action=pendingProjectAction.value;
  if(!action||deletingProjectId.value!==undefined)return;
  deletingProjectId.value=action.project.id;
  try{
    if(action.mode==="archive"){
      await api.archiveProject(action.project.id,true);
      notify("success",tr("工作空间已归档，Asset 与 Strix 历史均完整保留","Workspace archived; Asset and Strix history was preserved"));
    }else{
      await api.deleteProject(action.project.id);
      if(selectedProjectId.value===action.project.id){selectedProjectId.value=undefined;localStorage.removeItem("oviraptor-project");}
      notify("success",tr("空工作空间已删除","Empty workspace deleted"));
    }
    pendingProjectAction.value=undefined;
    await refresh();
  }catch(error){notify("error",String(error));}
  finally{deletingProjectId.value=undefined;}
}

async function toggleArchive(project:Project){
  try { await api.archiveProject(project.id,project.status==="active"); await refresh(); }
  catch(error){ notify("error",String(error)); }
}

async function autoSyncWorkers(){
  if(workerAutoSyncRunning)return;
  workerAutoSyncRunning=true;
  try{
    const nodes=await api.listWorkerNodes();
    // Import one potentially large Worker bundle at a time. Parallel imports
    // compete for SQLite and memory with active scans and previously caused a
    // burst exactly when a suspended macOS window became visible again.
    for(const node of nodes.filter(node=>node.enabled)){
      try{await api.syncWorkerNode(node.id);}catch{/* 节点错误已写入节点状态。 */}
    }
  }catch{/* 尚未配置 Worker 时保持安静。 */}
  finally{workerAutoSyncRunning=false;}
}

onMounted(async()=>{
  unlistenEnvironmentInstall=await listen<{stage:string;stream:string;message:string}>("environment-install-log",event=>{
    environmentInstallLogs.value.push({...event.payload,time:new Date().toLocaleTimeString()});
    if(event.payload.stream==="error") environmentInstallState.value="error";
    if(event.payload.stage==="complete"&&event.payload.stream==="success") environmentInstallState.value="success";
    nextTick(()=>{
      const output=environmentInstallConsole.value?.querySelector("pre");
      if(output) output.scrollTop=output.scrollHeight;
    });
  });
  void api.startupStatus().then(value=>startup.value=value).catch(error=>notify("error",String(error)));
  await refresh();
  void loadBrandIcon();
  strixUpdateTimer=setTimeout(()=>void checkStrixUpdate(false,false),5000);
  void autoSyncWorkers();
  workerSyncTimer=setInterval(autoSyncWorkers,5*60*1000);
  sentinelLogTimer=setInterval(pollSentinelActivity,8000);
  unlisten=await listen<JobProgressEvent>("job-progress",event=>{
    const progress=event.payload; const run=runs.value.find(item=>item.id===progress.runId);
    if(run){run.status=progress.status;run.stage=progress.stage;run.progress=progress.progress;}
    if(["completed","failed","cancelled"].includes(progress.status)){ notify(progress.status==="completed"?"success":"error",progress.message);refresh(); }
  });
  unlistenTray=await listen<string>("tray-navigate",event=>navigateFromTray(event.payload));
});
onUnmounted(()=>{unlisten?.();unlistenTray?.();unlistenEnvironmentInstall?.();if(workerSyncTimer)clearInterval(workerSyncTimer);if(sentinelLogTimer)clearInterval(sentinelLogTimer);if(strixUpdateTimer)clearTimeout(strixUpdateTimer)});
</script>

<template>
  <div class="app-shell" :class="[{ 'sidebar-collapsed': sidebarCollapsed },`theme-${themePreset}`]" :style="{'--blue':accentColor}">
    <aside class="sidebar">
      <div class="brand"><div class="brand-mark"><img v-if="brandIconUrl" :src="brandIconUrl" alt="Oviraptor" /></div><div class="brand-copy"><strong>{{t.appName}}</strong><span>Eating • Taking</span></div></div>
      <div class="module-switcher" aria-label="Product modules">
        <button v-if="hackerOneEnabled" :class="{active:activeModule==='hackerone'}" @click="switchModule('hackerone')"><span>H1</span><b>HackerOne</b></button>
        <button :class="{active:activeModule==='asset'}" @click="switchModule('asset')"><span>◈</span><b>Asset</b></button>
        <button :class="{active:activeModule==='sentinel'}" @click="switchModule('sentinel')"><span>◈</span><b>Strix</b></button>
      </div>
      <button class="sidebar-toggle" @click="sidebarCollapsed=!sidebarCollapsed"><Menu :size="17" /></button>
      <nav class="nav-list">
        <button v-for="item in nav" :key="item.key" :class="{active:activeView===item.key}" @click="navigate(item.key)">
          <component :is="item.icon" :size="18" /><span>{{item.label}}</span><em v-if="item.badge">{{item.badge}}</em>
        </button>
        <template v-if="activeModule==='sentinel'">
          <button v-for="item in sentinelNav" :key="item.key" :class="{active:sentinelMenu===item.key}" @click="openSentinelSection(item)"><component :is="item.icon" :size="18"/><span>{{item.label}}</span><em v-if="'badge' in item&&item.badge">{{item.badge}}</em></button>
        </template>
      </nav>
      <div class="sidebar-fixed-links">
        <button :class="{active:activeView==='logs'}" @click="navigate('logs')"><TerminalSquare :size="17" /><span>{{t.logs}}</span></button>
        <button :class="{active:activeView==='settings'}" @click="navigate('settings')"><Settings2 :size="17" /><span>{{t.settings}}</span></button>
      </div>
      <div class="sidebar-project">
        <span class="sidebar-label">CURRENT WORKSPACE</span>
        <button class="current-project" :title="tr('管理项目与范围','Manage projects and scope')" @click="navigate('projects')"><span class="project-dot"></span><span>{{selectedProject?.name||t.allProjects}}</span><ChevronDown :size="14" /></button>
        <button class="sidebar-create-workspace" @click="openProject()"><Plus :size="15" />{{tr('新建工作空间','New workspace')}}</button>
      </div>
      <footer class="sidebar-footer"><button :title="tr('查看更新说明','View release notes')" @click="releaseNotesDialog=true">v{{packageInfo.version}}</button><ShieldCheck :size="16" /><span>Local-first · SQLite</span></footer>
    </aside>

    <main class="main-area">
      <header class="topbar">
        <div class="page-heading"><span>{{selectedProject?.name||t.allProjects}}</span><h1>{{viewTitle}}</h1></div>
        <div class="topbar-actions">
          <div v-if="activeModule==='asset'&&!['logs','settings'].includes(activeView)" class="global-search"><Search :size="16" /><input v-model="globalSearch" :placeholder="tr('查询资产','Search assets')" @keyup.enter="doGlobalSearch" /></div>
          <div v-else-if="activeView==='sentinel'" class="global-search sentinel-global-search"><Search :size="16" /><input v-model="sentinelSearch" :placeholder="tr('搜索公司、URL、源码或任务','Search company, URL, source or task')" @keyup.enter="doGlobalSearch" /></div>
          <div class="project-switcher-group"><select class="project-switcher" :value="selectedProjectId" @change="selectProject(Number(($event.target as HTMLSelectElement).value)||undefined)"><option value="">{{t.allProjects}}</option><option v-for="project in projects" :key="project.id" :value="project.id">{{project.name}}{{project.status==='archived'?tr('（已归档）',' (archived)'):''}}</option></select><button class="workspace-create-cta" :title="tr('新建公共工作空间','Create shared workspace')" @click="openProject()"><Plus :size="16" /><span>{{tr('新建工作空间','New workspace')}}</span></button></div>
          <button class="top-icon" @click="setLocale(locale==='zh'?'en':'zh')"><Languages :size="18" /><span>{{locale==='zh'?'EN':'中'}}</span></button>
          <div class="theme-picker"><button class="top-icon" :title="tr('主题与强调色','Theme and accent')" @click="showTheme=!showTheme"><Palette :size="18" /></button><div v-if="showTheme" class="theme-popover panel"><strong>{{tr('界面主题','Interface theme')}}</strong><button v-for="item in [['cloud',tr('云白','Cloud')],['codex',tr('Codex','Codex')]]" :key="item[0]" :class="{active:themePreset===item[0]}" @click="setTheme(item[0])">{{item[1]}}</button><label><span>{{tr('自定义强调色','Custom accent')}}</span><input v-model="accentColor" type="color" @change="saveAccent" /></label></div></div>
          <button class="top-icon" :title="tr('查看任务与 Scope 变化','View task and scope changes')" @click="navigate(h1HasChanges?'hackerone':'tasks')"><Bell :size="18" /><i v-if="stats?.runningJobs||h1HasChanges"></i></button>
        </div>
      </header>

      <div class="content-area">
        <section v-if="!loading&&startupNotice&&(startup.interruptedJobs.length||startup.staleProjects.length)" class="startup-alert panel">
          <div class="startup-alert-icon"><CircleAlert :size="20" /></div>
          <div class="startup-alert-copy">
            <strong>{{startup.interruptedJobs.length?tr(`发现 ${startup.interruptedJobs.length} 个中断任务`,`Found ${startup.interruptedJobs.length} interrupted jobs`):tr('项目更新提醒','Project update reminder')}}</strong>
            <span>{{tr('自动更新需要电脑和应用保持运行；关闭窗口后应用会留在 macOS 状态栏。','Automatic updates require the computer and app to stay running; closing the window keeps the app in the macOS menu bar.')}}</span>
            <div v-if="startup.interruptedJobs.length" class="startup-items"><button v-for="job in startup.interruptedJobs.slice(0,4)" :key="job.runId" @click="resumeInterrupted(job)"><PlayCircle :size="14" /> {{job.projectName}} · {{tr('继续更新','Resume')}}</button></div>
            <div v-if="startup.staleProjects.length" class="startup-items"><button v-for="project in startup.staleProjects.slice(0,4)" :key="project.projectId" @click="quickStartProject(project.projectId)"><RefreshCw :size="14" /> {{project.projectName}} · {{project.daysSinceUpdate==null?tr('尚未更新','Never updated'):tr(`${project.daysSinceUpdate} 天未更新`,`${project.daysSinceUpdate} days old`)}}</button></div>
          </div>
          <button class="icon-button subtle" @click="startupNotice=false"><X :size="15" /></button>
        </section>
        <section v-if="!loading&&!strixUpdateDismissed&&strixUpdate?.updateAvailable" class="startup-alert strix-update-alert panel">
          <div class="startup-alert-icon"><RefreshCw :size="20" /></div>
          <div class="startup-alert-copy">
            <strong>{{tr(`Strix ${strixUpdate.latestVersion} 可用`,`Strix ${strixUpdate.latestVersion} is available`)}}</strong>
            <span>{{tr(`当前 ${strixUpdate.currentVersion}。更新检查在首屏加载完成后异步执行，不会阻塞数据库或页面启动。`,`Current ${strixUpdate.currentVersion}. Update checks run asynchronously after the first screen and never block database or page startup.`)}}</span>
            <div class="startup-items"><button @click="openStrixUpdate"><TerminalSquare :size="14" /> {{tr('前往运行环境升级','Open runtime updater')}}</button></div>
          </div>
          <button class="icon-button subtle" @click="strixUpdateDismissed=true"><X :size="15" /></button>
        </section>
        <div v-if="loading" class="app-loading"><div class="loader-ring"></div><span>{{tr('正在初始化本地资产数据库…','Initializing local asset database…')}}</span></div>

        <template v-else-if="activeView==='dashboard'">
          <div class="hero-row">
            <div><span class="eyebrow">OVERVIEW</span><h2>{{selectedProject?tr(`${selectedProject.name} 概览`,`${selectedProject.name} overview`):tr('所有项目资产概览','All-project asset overview')}}</h2><p>{{tr('数据保存在本地 SQLite；仅变化事件进入历史，不重复保存完整镜像。','Data stays in local SQLite; history stores changes instead of duplicate full snapshots.')}}</p></div>
            <div class="hero-actions"><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button><button v-if="selectedProject?.status==='archived'" class="button warning" @click="toggleArchive(selectedProject)">{{tr('恢复工作空间后继续','Restore workspace to continue')}}</button><button v-else-if="selectedProjectId" class="button secondary" @click="quickStartProject(selectedProjectId,undefined,undefined,'reprobe')"><RefreshCw :size="16" /> {{tr('复测现有资产','Re-probe existing')}}</button><button v-if="selectedProjectId&&selectedProject?.status!=='archived'" class="button secondary" @click="quickStartProject(selectedProjectId)"><RefreshCw :size="16" /> {{tr('重新采集并探测','Collect and probe')}}</button><button class="button primary" @click="navigate('query')"><Plus :size="16" /> {{t.newQuery}}</button></div>
          </div>
          <div class="stats-grid">
            <article class="stat-card"><div class="stat-icon blue"><Database :size="20" /></div><span>{{tr('资产总数','Total assets')}}</span><strong>{{(stats?.assetCount||0).toLocaleString()}}</strong><small>{{tr(`${stats?.projectCount||0} 个活跃项目`,`${stats?.projectCount||0} active projects`)}}</small></article>
            <article class="stat-card"><div class="stat-icon green"><CheckCircle2 :size="20" /></div><span>{{tr('浏览器可访问','Browser accessible')}}</span><strong>{{(stats?.aliveCount||0).toLocaleString()}}</strong><small>{{tr('不包含仅 TCP 端口存活；旧数据需复测','Excludes TCP-only; re-probe legacy data')}}</small></article>
            <article class="stat-card"><div class="stat-icon violet"><ListFilter :size="20" /></div><span>{{tr('待人工确认','Pending review')}}</span><strong>{{(stats?.pendingCount||0).toLocaleString()}}</strong><small>{{tr('P1/P2/P3 复核队列','P1/P2/P3 review queue')}}</small></article>
            <button class="stat-card stat-button" @click="navigate('quarantine')"><div class="stat-icon amber"><CircleAlert :size="20" /></div><span>{{tr('内容隔离','Content blocked')}}</span><strong>{{(stats?.blockedCount||0).toLocaleString()}}</strong><small>{{tr('点击进入隔离区查看','Open quarantine for review')}}</small></button>
          </div>
          <div class="dashboard-grid">
            <section class="panel recent-changes"><div class="panel-heading"><div><span class="eyebrow">CHANGE FEED</span><h3>{{tr('最近变化','Recent changes')}}</h3></div><button class="text-button" @click="navigate('changes')">{{tr('查看全部','View all')}}</button></div>
              <div v-if="events.length" class="change-list"><div v-for="event in events.slice(0,8)" :key="event.id" class="change-row"><span class="change-type" :class="`event-${event.eventType}`">{{eventLabel(event.eventType)}}</span><div><strong>{{event.company||event.host||event.assetKey}}</strong><span>{{event.summary}}</span></div><time>{{event.createdAt}}</time></div></div><div v-else class="empty-state small">{{tr('完成首次任务后，这里会显示新增和变化资产','New and changed assets appear here after the first job')}}</div>
            </section>
            <section class="panel recent-jobs"><div class="panel-heading"><div><span class="eyebrow">JOBS</span><h3>{{tr('任务状态','Job status')}}</h3></div><button class="text-button" @click="navigate('tasks')">{{tr('任务中心','Job center')}}</button></div>
              <div v-if="runs.length" class="job-mini-list"><div v-for="run in runs.slice(0,6)" :key="run.id" class="job-mini"><div class="job-mini-head"><span class="status-dot" :class="statusTone(run.status)"></span><strong>{{run.name}}</strong><em>{{stageLabel(run.stage)}}</em></div><div class="progress-track"><i :style="{width:`${run.progress}%`}"></i></div><small>{{run.projectName}} · {{run.createdAt}}</small></div></div><div v-else class="empty-state small">{{tr('暂无运行记录','No job history')}}</div>
            </section>
          </div>
        </template>

        <template v-else-if="activeView==='projects'">
          <div class="section-toolbar"><div><span class="eyebrow">SHARED WORKSPACES</span><h2>{{tr('项目与范围','Projects and scope')}}</h2><p>{{tr('一个工作空间统一承载 Asset 范围、Strix 扫描、证据、漏洞结论和知识沉淀。','One workspace owns Asset scope, Strix scans, evidence, conclusions, and learned knowledge.')}}</p></div><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('新建工作空间','New workspace')}}</button></div>
          <div v-if="projects.length" class="project-grid shared-project-grid">
            <article v-for="project in projects" :key="project.id" class="project-card shared-project-card" :class="{selected:selectedProjectId===project.id,archived:project.status==='archived'}">
              <header><div class="project-icon"><FolderKanban :size="20" /></div><span class="state-pill">{{project.status==='active'?tr('活跃','Active'):tr('已归档','Archived')}}</span></header>
              <h3>{{project.name}}</h3><p>{{project.description||tr('暂无范围与授权说明','No scope or authorization note')}}</p>
              <div class="project-metrics shared-project-metrics"><span><strong>{{project.assetCount.toLocaleString()}}</strong>{{tr('资产','Assets')}}</span><span><strong>{{project.scanCount.toLocaleString()}}</strong>Strix</span><span><strong>{{project.vulnerabilityCount.toLocaleString()}}</strong>{{tr('漏洞','Findings')}}</span><span><strong>{{project.activeFuseCount.toLocaleString()}}</strong>{{tr('待处置','Stopped')}}</span></div>
              <div class="project-entry-actions"><button class="button ghost compact" @click="openProjectWorkspace(project,'asset')"><Database :size="14" />Asset</button><button class="button secondary compact" @click="openProjectWorkspace(project,'sentinel')"><ShieldCheck :size="14" />Strix</button></div>
              <footer><span>{{tr('最近活动','Last activity')}} {{projectActivity(project)}}</span><div><button class="text-button" @click="openProject(project)">{{tr('编辑','Edit')}}</button><button class="text-button" @click="toggleArchive(project)">{{project.status==='active'?tr('归档','Archive'):tr('恢复','Restore')}}</button><button class="text-button danger-text" :disabled="deletingProjectId!==undefined" @click="removeProject(project)">{{tr('删除','Delete')}}</button></div></footer>
              <InlineConfirm v-if="pendingProjectAction?.project.id===project.id" :title="pendingProjectAction.mode==='archive'?tr('该工作空间有关联数据，改为归档？','This workspace has linked data. Archive it?'):tr('删除这个空工作空间？','Delete this empty workspace?')" :detail="pendingProjectAction.mode==='archive'?`${projectImpactText(pendingProjectAction.impact)}。归档后禁止创建新任务，但历史和结论继续可查。`:tr('它没有资产、扫描、证据或知识记录，删除后无法恢复。','It has no assets, scans, evidence, or knowledge and cannot be recovered.')" :confirm-text="pendingProjectAction.mode==='archive'?tr('归档并保留历史','Archive and preserve history'):tr('确认删除','Delete')" :busy-text="pendingProjectAction.mode==='archive'?tr('归档中…','Archiving…'):tr('删除中…','Deleting…')" :tone="pendingProjectAction.mode==='archive'?'warning':'danger'" :busy="deletingProjectId===project.id" @cancel="pendingProjectAction=undefined" @confirm="confirmProjectAction" />
            </article>
          </div>
          <div v-else class="first-run panel"><div class="first-run-icon"><FolderKanban :size="28" /></div><h2>{{tr('创建第一个公共工作空间','Create your first shared workspace')}}</h2><p>{{tr('无需先采集资产；也可以直接创建 Strix Web、代码审计或灰盒任务。','Asset collection is optional; you can start directly with Strix web, code, or grey-box tasks.')}}</p><button class="button primary" @click="openProject()"><Plus :size="16" /> {{tr('创建工作空间','Create workspace')}}</button></div>
        </template>

        <QueryPanel v-else-if="activeView==='query'" :projects="projects" :profiles="profiles" :selected-project-id="selectedProjectId" @create-project="openProject()" @project-change="selectProject" @started="navigate('tasks')" @notify="notify" />
        <AssetWorkspace v-else-if="activeView==='assets'" :projects="projects" :selected-project-id="selectedProjectId" :initial-search="globalSearch" @reprobe="quickStartProject($event,undefined,undefined,'reprobe')" @notify="notify" />

        <template v-else-if="activeView==='quarantine'">
          <div class="section-toolbar"><div><span class="eyebrow">QUARANTINE</span><h2>{{tr('内容隔离区','Content quarantine')}}</h2><p>{{tr('集中查看自动规则隔离的赌博、色情和自定义命中；数据不会被物理删除。','Review gambling, adult and custom-rule matches; quarantined data is never physically deleted.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <AssetWorkspace :projects="projects" :selected-project-id="selectedProjectId" quarantine-only @notify="notify" />
        </template>

        <HackerOneBoard v-else-if="activeView==='hackerone'" :projects="projects" :profiles="profiles" @notify="notify" />
        <div v-else-if="activeView==='sentinel'" class="sentinel-persistent-slot" aria-hidden="true"></div>

        <template v-else-if="activeView==='changes'">
          <div class="section-toolbar"><div><span class="eyebrow">DIFF & HISTORY</span><h2>{{tr('变化事件','Change events')}}</h2><p>{{tr('只记录新增、关键字段变化、未再次发现和人工操作，不重复保存完整快照。','Records new assets, key changes, assets not seen again and manual actions without duplicate full snapshots.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <section class="panel list-panel"><div class="event-table-head"><span>{{tr('类型','Type')}}</span><span>{{tr('资产','Asset')}}</span><span>{{tr('变化说明','Change')}}</span><span>{{tr('任务','Job')}}</span><span>{{tr('时间','Time')}}</span></div><div v-for="event in events" :key="event.id" class="event-table-row"><span><em class="change-type" :class="`event-${event.eventType}`">{{eventLabel(event.eventType)}}</em></span><span><strong>{{event.company||'—'}}</strong><small>{{event.host||event.assetKey}}</small></span><span>{{event.summary}}</span><span>#{{event.runId||'—'}}</span><time>{{event.createdAt}}</time></div><div v-if="!events.length" class="empty-state">{{tr('暂无变化事件','No change events')}}</div></section>
        </template>

        <template v-else-if="activeView==='tasks'">
          <div class="section-toolbar"><div><span class="eyebrow">JOB CENTER</span><h2>{{tr('任务中心','Job center')}}</h2><p>{{tr('长时间任务可独立运行、查看结构化日志并安全取消。','Long jobs run independently with structured logs and safe cancellation.')}}</p></div><div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button><button class="button primary" @click="navigate('query')"><Plus :size="16" /> {{tr('新建任务','New job')}}</button></div></div>
          <div class="run-list"><article v-for="run in runs" :key="run.id" class="run-card panel"><div class="run-status-mark" :class="statusTone(run.status)"><component :is="run.status==='completed'?CheckCircle2:['failed','interrupted'].includes(run.status)?CircleAlert:Clock3" :size="20" /></div><div class="run-main"><div class="run-title"><strong>{{run.name}}</strong><span class="run-id">#{{run.id}}</span><span class="run-stage">{{stageLabel(run.stage)}}</span></div><p>{{run.projectName}} · {{tr('创建于','created')}} {{run.createdAt}}</p><div class="run-progress"><div class="progress-track"><i :style="{width:`${run.progress}%`}"></i></div><span>{{Math.round(run.progress)}}%</span></div><small v-if="run.error" class="run-error">{{run.error}}</small></div><div class="run-side"><span class="status-chip" :class="statusTone(run.status)">{{stageLabel(run.status)}}</span><button v-if="run.status==='interrupted'||(run.pipeline==='reprobe'&&['failed','cancelled'].includes(run.status))" class="button secondary compact" @click="resumeInterrupted({runId:run.id,projectId:run.projectId,projectName:run.projectName,profileId:run.profileId,name:run.name,pipeline:run.pipeline,createdAt:run.createdAt})">{{tr('从断点继续','Resume checkpoint')}}</button><button v-if="['running','queued','cancel_requested'].includes(run.status)" class="button danger compact" @click="api.cancelJob(run.id).then(()=>refresh()).catch(e=>notify('error',String(e)))">{{tr('取消','Cancel')}}</button><button class="button ghost compact" @click="navigate('logs')">{{tr('日志','Logs')}}</button></div></article><div v-if="!runs.length" class="empty-state panel">{{tr('暂无任务','No jobs')}}</div></div>
        </template>

        <template v-else-if="activeView==='logs'">
          <div class="section-toolbar"><div><span class="eyebrow">AUDIT TRAIL · {{activeModule.toUpperCase()}}</span><h2>{{activeModule==='sentinel'?tr('扫描记录','Scan activity'):activeModule==='hackerone'?tr('HackerOne 同步记录','HackerOne sync activity'):tr('资产采集与操作日志','Asset collection activity')}}</h2><p>{{activeModule==='sentinel'?tr('只显示扫描状态、任务类型、Token 和检查点，不在 URL 概要中堆叠原始日志。','Shows scan status, type, tokens and checkpoints without dumping raw logs into URL summaries.'):tr('当前模块的运行记录；敏感配置不会在界面显示。','Activity for the current module; secrets are not shown.')}}</p></div><button class="button ghost" @click="refresh"><RefreshCw :size="16" /> {{t.refresh}}</button></div>
          <section v-if="activeModule==='sentinel'" class="panel module-activity-list"><article v-for="scan in sentinelLogScans" :key="scan.id"><span class="run-status-mark" :class="statusTone(scan.status)"><Activity :size="18"/></span><div><strong>{{scan.taskName||scan.projectName}}</strong><small>{{scan.scanType||'web'}} · {{scan.projectName}} · {{scan.id}}</small><p>{{sentinelCheckpointSummary(scan)}}</p><details v-if="sentinelErrorDetail(scan)" class="activity-error-detail"><summary><CircleAlert :size="13"/>{{tr('报错细节','Error details')}}</summary><pre>{{sentinelErrorDetail(scan)}}</pre></details></div><div class="activity-metric"><b>{{scan.totalTokens.toLocaleString()}}</b><span>Token</span><time>{{scan.updatedAt}}</time></div></article><div v-if="!sentinelLogScans.length" class="empty-state">{{tr('暂无扫描记录','No scan activity')}}</div></section>
          <section v-else-if="activeModule==='hackerone'" class="panel module-activity-list"><article v-for="event in hackerOneLogEvents" :key="event.id"><span class="run-status-mark success"><ShieldCheck :size="18"/></span><div><strong>{{event.programHandle}}</strong><small>{{event.eventType}}</small><p>{{event.summary}}</p></div><time>{{event.createdAt}}</time></article><div v-if="!hackerOneLogEvents.length" class="empty-state">{{tr('暂无 HackerOne 同步记录','No HackerOne sync activity')}}</div></section>
          <section v-else class="panel log-panel"><div v-for="log in logs" :key="log.id" class="log-row"><time>{{log.createdAt}}</time><span class="log-level" :class="`log-${log.level}`">{{log.level}}</span><span class="log-stage">{{log.stage||'system'}}</span><code>{{log.message}}</code><em v-if="log.runId">#{{log.runId}}</em></div><div v-if="!logs.length" class="empty-state">{{tr('暂无日志','No logs')}}</div></section>
        </template>

        <template v-else-if="activeView==='settings'">
          <div class="section-toolbar"><div><span class="eyebrow">CONFIGURATION CENTER</span><h2>{{tr('配置中心','Configuration center')}}</h2><p>{{tr('运行方案、远程 Worker 和本机环境分区管理，避免创建无内容的空配置。','Manage runtime profiles, remote Workers and the local environment without creating empty profiles.')}}</p></div><button class="button ghost" @click="appSettingsDialog=true"><AppWindow :size="16" /> {{tr('应用设置','App settings')}}</button></div>
          <nav class="config-center-nav">
            <button :class="{active:configSection==='profiles'}" @click="configSection='profiles'"><SlidersHorizontal :size="18"/><span><b>{{tr('运行方案','Runtime profiles')}}</b><small>{{tr('账号、模型与扫描参数','Accounts, models and scan policy')}}</small></span></button>
            <button :class="{active:configSection==='workers'}" @click="configSection='workers'"><Server :size="18"/><span><b>Worker {{tr('节点','nodes')}}</b><small>{{tr('Intel Mac / Windows 远程执行','Remote execution on Intel Mac / Windows')}}</small></span></button>
            <button :class="{active:configSection==='runtime'}" @click="configSection='runtime'"><TerminalSquare :size="18"/><span><b>{{tr('运行环境','Runtime environment')}}</b><small>{{tr('检测、安装与实时输出','Check, install and view live output')}}</small></span></button>
          </nav>

          <template v-if="configSection==='profiles'">
            <section class="app-settings-summary panel"><div class="profile-icon"><AppWindow :size="19" /></div><div><strong>{{tr('更新与后台设置','Update and background')}}</strong><span>{{tr(`超过 ${appSettings.reminderDays} 天未更新时提醒 · ${appSettings.customIcon?'自定义图标':'默认图标'}`,`Remind after ${appSettings.reminderDays} days · ${appSettings.customIcon?'custom icon':'default icon'}`)}}</span></div><button class="button ghost compact" @click="appSettingsDialog=true">{{tr('修改','Edit')}}</button></section>
            <div class="profile-section-heading"><div><h3>{{tr('运行方案','Runtime profiles')}}</h3><p>{{tr('系统默认方案始终保留；需要新方案时从默认方案复制，避免产生空配置。','The system default is permanent. Create new profiles by copying it so they always start complete.')}}</p></div><button class="button primary" @click="cloneDefaultProfile"><Plus :size="16"/>{{tr('从默认方案创建','Create from default')}}</button></div>
            <div class="profile-grid"><article v-for="profile in profiles" :key="profile.id" class="profile-card panel"><header><div class="profile-icon"><Settings2 :size="19" /></div><span v-if="profile.isDefault" class="default-pill">SYSTEM DEFAULT</span></header><h3>{{profile.name}}</h3><p>{{profile.description}}</p><dl><div><dt>Python</dt><dd>{{profile.settings.pythonExecutable}}</dd></div><div><dt>Scripts</dt><dd>{{profile.settings.scriptsDirectory||tr('内置脚本','Bundled')}}</dd></div><div><dt>Probe</dt><dd>{{profile.settings.priorityRate}} r/s · {{profile.settings.workers}} workers</dd></div><div><dt>Rules</dt><dd>{{(profile.settings.gamblingKeywords?.length||0)+(profile.settings.pornKeywords?.length||0)}} keywords</dd></div></dl><footer><span>{{tr('更新于','Updated')}} {{profile.updatedAt}}</span><div><button class="button ghost compact" @click="openProfile(profile)">{{tr('编辑','Edit')}}</button><button v-if="!profile.isDefault" class="button danger compact" @click="removeProfile(profile)"><Trash2 :size="13"/>{{deletingProfileId===profile.id?tr('确认删除','Confirm delete'):tr('删除','Delete')}}</button></div></footer></article></div>
          </template>

          <WorkerSettingsPanel v-else-if="configSection==='workers'" @message="notify" />

          <section v-else class="panel environment-card">
            <div class="panel-heading">
              <div><span class="eyebrow">RUNTIME CHECK</span><h3>{{tr('运行环境检测','Environment check')}}</h3><p>{{tr('检查应用实际使用的 Python、Node.js、模块、redis-cli、Strix、Docker CLI 与 Docker daemon；不会上传数据。','Check the Python, Node.js, modules, redis-cli, Strix, Docker CLI and Docker daemon used by the app; nothing is uploaded.')}}</p></div>
              <div class="hero-actions">
                <button class="button ghost compact" :disabled="environmentChecking||environmentInstalling" @click="checkEnvironment"><RefreshCw :size="14" :class="{spinning:environmentChecking}" /> {{environmentChecking?tr('检测中…','Checking…'):tr('检测','Check')}}</button>
                <button v-if="environment&&['macOS','Windows'].includes(environment.os)" class="button secondary compact" :disabled="environmentChecking||environmentInstalling" @click="installEnvironment"><RefreshCw v-if="environmentInstalling" :size="14" class="spinning" /> {{environmentInstalling?tr('安装中，请查看日志','Installing — see log'):environment.os==='Windows'?tr('Windows 自动安装','Install on Windows'):tr('Mac 自动安装','Install on Mac')}}</button>
              </div>
            </div>
            <div v-if="environment" class="environment-grid"><div><span>OS</span><strong>{{environment.os}} · {{environment.arch}}</strong></div><div><span>Python</span><strong :title="environment.python">{{environment.python}}</strong></div><div><span>Node.js</span><strong :title="environment.node">{{environment.node}}</strong></div><div><span>redis-cli</span><strong :title="environment.redisCli">{{environment.redisCli}}</strong></div><div><span>Strix CLI</span><strong :title="environment.strixCli">{{environment.strixCli}}</strong></div><div><span>Docker CLI</span><strong :title="environment.dockerCli">{{environment.dockerCli}}</strong></div><div><span>Docker daemon</span><strong :class="environment.dockerDaemon.startsWith('可用')?'env-ok':'env-bad'" :title="environment.dockerDaemon">{{environment.dockerDaemon}}</strong></div><div v-for="dep in environment.dependencies" :key="dep.name"><span>{{dep.name}} · {{dep.command}}</span><strong :class="dep.available?'env-ok':'env-bad'" :title="dep.detail">{{dep.available?'OK':'缺失'}} · {{dep.version}}</strong><small v-if="!dep.available">{{dep.detail}}</small></div></div>
            <div v-else class="empty-state small">{{tr('点击检测查看依赖状态','Click Check to inspect dependencies')}}</div>
            <div class="strix-updater-card">
              <div class="strix-updater-status" :class="{available: strixUpdate?.updateAvailable, error: Boolean(strixUpdate?.checkError)}">
                <span><RefreshCw :size="16" /></span>
                <div>
                  <strong>{{strixUpdate?.checkError?tr('Strix 更新检查失败','Strix update check failed'):strixUpdate?.updateAvailable?tr('发现 Strix 新版本','Strix update available'):strixUpdate?.latestVersion?tr('Strix 已是最新版本','Strix is up to date'):tr('Strix 版本更新','Strix version update')}}</strong>
                  <small v-if="strixUpdate?.checkError">{{strixUpdate.checkError}}</small>
                  <small v-else-if="strixUpdate">{{tr('本机','Local')}} {{strixUpdate.currentVersion||tr('未安装','Not installed')}} · {{tr('最新','Latest')}} {{strixUpdate.latestVersion||'—'}} · {{strixUpdate.executable||tr('未找到可执行文件','Executable not found')}}</small>
                  <small v-else>{{tr('启动后延迟 5 秒在后台检查；结果缓存 12 小时，不占用首屏加载时间。','Checks five seconds after startup in the background and caches results for 12 hours.') }}</small>
                </div>
              </div>
              <div class="strix-updater-actions">
                <button class="button ghost compact" :disabled="strixUpdateChecking||strixUpdating||environmentInstalling" @click="checkStrixUpdate(true,true)"><RefreshCw :size="13" :class="{spinning:strixUpdateChecking}"/>{{strixUpdateChecking?tr('检查中…','Checking…'):tr('重新检查','Check now')}}</button>
                <button v-if="strixUpdate?.updateAvailable||strixUpdate&&!strixUpdate.installed" class="button primary compact" :disabled="strixUpdating||environmentInstalling" @click="updateStrix"><TerminalSquare :size="13"/>{{strixUpdating?tr('升级中，查看下方输出','Updating — see output'):tr('在 App 内升级','Update in App')}}</button>
              </div>
            </div>
            <div v-if="environmentInstallState!=='idle'||environmentInstallLogs.length" ref="environmentInstallConsole" class="environment-install-console">
              <header>
                <div><TerminalSquare :size="15" /><strong>{{tr('安装实时输出','Live installation output')}}</strong></div>
                <span :class="environmentInstallState">{{environmentInstallState==='running'?tr('执行中','RUNNING'):environmentInstallState==='success'?tr('已完成','SUCCESS'):environmentInstallState==='error'?tr('失败','FAILED'):tr('待执行','IDLE')}}</span>
              </header>
              <pre><code v-for="(line,index) in environmentInstallLogs" :key="`${index}-${line.time}`" :class="line.stream"><time>{{line.time}}</time><b>[{{line.stage}}]</b> {{line.message}}</code><code v-if="environmentInstalling||strixUpdating" class="pending">▌</code></pre>
              <p v-if="environmentInstallError">{{environmentInstallError}}</p>
            </div>
          </section>
        </template>
        <!-- The Strix result tree can contain thousands of reactive rows. Keep
             it mounted only while visible so macOS/WebKit does not have to
             restore and repaint a hidden second application after occlusion. -->
        <SentinelBoard v-if="!loading&&activeView==='sentinel'" :active="true" :projects="projects" :project-id="selectedProjectId" :section="sentinelSection" :result-view="sentinelResultView" :workbench-mode="sentinelWorkbenchMode" :search="sentinelSearch" @create-project="openProject()" @section-change="sentinelSection=$event" @alerts-change="sentinelAlerts=$event" @projects-change="refresh" @notify="notify" />
        <section v-if="activeView==='logs'&&activeModule==='sentinel'" class="panel sentinel-runner-console">
          <header class="sentinel-runner-console-header">
            <div><TerminalSquare :size="15"/><strong>{{tr('Strix 实时运行日志','Strix live runner log')}}</strong><span v-if="sentinelRunnerLogLoading">{{tr('读取中','Reading')}}</span></div>
            <select class="toolbar-select" :value="sentinelLogScanId" @change="selectSentinelLogScan(($event.target as HTMLSelectElement).value)"><option v-for="scan in sentinelLogScans" :key="scan.id" :value="scan.id">{{scan.taskName||scan.projectName}} · {{scan.status}} · {{scan.id}}</option></select>
          </header>
          <pre ref="sentinelRunnerConsole" class="sentinel-runner-log"><code v-for="(line,index) in sentinelRunnerLogs" :key="`${index}-${line}`">{{line}}</code><code v-if="!sentinelRunnerLogs.length" class="empty-log">{{tr('尚无 runner 日志；任务启动后会在这里实时显示每个步骤','No runner output yet; each worker step will appear here')}}</code></pre>
        </section>
      </div>
    </main>

    <ProjectDialog v-if="projectDialog" :project="editProject" @close="projectDialog=false;editProject=undefined" @saved="projectSaved" />
    <ConfigDialog v-if="configDialog" :profile="editProfile" @close="configDialog=false;editProfile=undefined" @saved="profileSaved" />
    <AppSettingsDialog v-if="appSettingsDialog" :settings="appSettings" @close="appSettingsDialog=false" @saved="appSettingsSaved" @icon-changed="loadBrandIcon" />
    <ReleaseNotesDialog v-if="releaseNotesDialog" :version="packageInfo.version" @close="releaseNotesDialog=false" />
    <div class="toast-stack"><div v-for="toast in toasts" :key="toast.id" class="toast" :class="toast.type"><CheckCircle2 v-if="toast.type==='success'" :size="18" /><CircleAlert v-else-if="toast.type==='error'" :size="18" /><Bell v-else :size="18" /><span>{{toast.text}}</span><button @click="toasts=toasts.filter(t=>t.id!==toast.id)"><X :size="14" /></button></div></div>
  </div>
</template>
