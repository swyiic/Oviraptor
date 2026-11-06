import type { Messages } from "./zh";

export const en: Messages = {
  app: { name: "Oviraptor" },
  common: {
    refresh: "Refresh", export: "Export", save: "Save", cancel: "Cancel", confirm: "Confirm", close: "Close",
    confirmDelete: "Delete", deleting: "Deleting…", empty: "No data", language: "Language",
  },
  shell: {
    workspace: "Workspace", allWorkspaces: "All workspaces", newWorkspace: "New workspace",
    workspacesAndScope: "Workspaces and scope", manageWorkspaces: "Manage workspaces and scope", chooseWorkspace: "Choose workspace",
    logs: "Logs", settings: "Settings", find: "Find", appOptions: "App options", builtInPlugins: "Built-in plugins",
  },
  modules: { chat: "Chat", asset: "Assets", sentinel: "Scanning", hackerone: "HackerOne" },
  pages: {
    asset: {
      dashboard: "Overview", tasks: "Collection jobs", assets: "Asset list", seeds: "Discovery seeds", ownership: "Ownership review",
      exposure: "Public exposure", changes: "Changes", quarantine: "Quarantine", query: "Discover assets", logs: "Asset logs",
    },
    sentinel: {
      overview: "Overview", queue: "Scan tasks", urls: "Results and evidence", dialog: "Run details", fuse: "Stopped targets",
      new_scan: "New scan", knowledge: "Knowledge base", logs: "Scan logs", validations: "Manual validation queue",
    },
  },
  settings: {
    groups: { modelsSkills: "Models & skills", assets: "Assets", scanning: "Scanning", machineNetwork: "Machine & network", accessPlugins: "Access & plugins" },
    items: {
      models: "Models", skills: "Skills", mcp: "MCP", agentKnowledge: "Memory & techniques", assetApis: "Asset APIs", collection: "Asset collection", probe: "Asset probing",
      contentRules: "Content classification", pageRules: "Page rules", agentPolicy: "Scan agent policy", codeRules: "Code rule libraries",
      fingerprints: "Fingerprint library", businessNetwork: "Business networking", workers: "Worker nodes", environment: "Environment",
      dataPermissions: "Data permissions", apps: "Sidebar & apps",
    },
  },
  scanning: {
    vulnerabilities: "vulns", targets: "Targets", task: "Task", taskList: "Scan tasks", runDetails: "Run details",
    companyAssets: "Company assets", sourceContext: "Source context", newScan: "New scan",
    overviewIntro: "Start with each recent task's vulnerability count and the opportunities ready to verify; scans call a model only after the deterministic pipeline finds high-value candidates.",
    overviewMore: "Stats, cost and workflow notes",
    resultsOneTask: "Results and evidence shows one task at a time. Open a task from Scan tasks or the cards below.",
    chooseTask: "Choose a task to review",
    taskListIntro: "The large number on each row is that task's published vulnerabilities. Click a task to open its Results and evidence; drafts open the confirmation panel.",
    process: {
      title: "Investigation process", intro: "How the task ran. Findings, fingerprints and APIs are in Results and evidence.",
      graph: "Investigation graph", opportunities: "Opportunities & next steps", trace: "Trace", attempts: "Attempts & cost", routing: "Routing & capabilities",
      informationGain: "Information gain {value}/100", loading: "Reading the run…", loadingTask: "Loading this task's run…",
      noTrace: "No saved trace for this task.", noRouting: "No routing or capability records for this target.",
    },
    row: { run: "Run", details: "Details", confirmScan: "Confirm scan", stopKeep: "Stop and keep", stopping: "Stopping",
      resumePartial: "Resume remaining", resume: "Resume", archive: "Archive", unarchive: "Unarchive", delete: "Delete",
      forceDelete: "Force delete", archived: "Archived", high: "High-value {count}", mode: "Mode: {mode}" },
    deleteTask: "Delete task “{name}”?",
    deleteActive: "This task is still running or waiting to exit, so the backend will refuse the delete. Pause it and confirm the run exited and requests are settled first; deleting never kills processes or clears files on disk.",
    deleteRecord: "Deletes the whole task record: linked URLs, parsed results, conversations and manual validations. A task with unknown requests or unsettled runs is kept and the delete is refused. Source files and task artifacts stay on disk. To tidy the list only, archive instead.",
    forceDeleteTitle: "Force delete crashed task “{name}”?",
    forceDeleteDetail: "The normal delete was refused: {reason}. This task crashed without settling (it left a budget reservation or allocation open). Force delete skips settlement checks and removes the task and its records; the overview token total drops accordingly. Financial audit records are archived unchanged (marked as force-deleted). The local process must still have exited; no process is killed and no files are removed. This cannot be undone.",
    pauseTitle: "Request a pause?",
    pauseDetail: "The task is marked Paused once its worker exits; saved evidence, unknown target effects and unconfirmed cleanup records are kept.",
    pauseConfirm: "Request pause",
  },
};
