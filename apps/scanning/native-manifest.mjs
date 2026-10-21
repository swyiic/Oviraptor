// Native execution is a versioned package. The Node v1 draft package remains
// separate and is never allowed to spawn a native executable implicitly.
export function createNativeManifest({ platform = process.platform, arch = process.arch } = {}) {
  const task = { type: 'object', properties: { taskId: { type: 'string', minLength: 1, maxLength: 128 } }, required: ['taskId'], additionalProperties: false };
  const empty = { type: 'object', properties: {}, additionalProperties: false };
  const workbench = { type: 'object', properties: {
    sourceRef: { type: 'string', minLength: 36, maxLength: 36 }, title: { type: 'string', minLength: 1, maxLength: 120 },
    scanType: { type: 'string', enum: ['code', 'greybox', 'cicd'] },
    urls: { type: 'array', maxItems: 128, items: { type: 'string', minLength: 1, maxLength: 2048 } },
    skillIds: { type: 'array', maxItems: 128, items: { type: 'integer', minimum: 1, maximum: Number.MAX_SAFE_INTEGER } },
    instruction: { type: 'string', maxLength: 16384 }, scanMode: { type: 'string', enum: ['quick', 'standard', 'deep'] },
    maxBudgetUsd: { type: 'number', minimum: 0, maximum: 10000 }, environment: { type: 'string', maxLength: 80 },
    maxCritical: { type: 'integer', minimum: 0, maximum: 10000 }, maxHigh: { type: 'integer', minimum: 0, maximum: 10000 }, blockRelease: { type: 'boolean' },
  }, required: ['sourceRef', 'title', 'scanType'], additionalProperties: false };
  const tools = [
    { name: 'scan_workbench_start', permission: 'investigation.execute', description: 'Start original Source/Greybox/CI work on an operator-imported immutable private snapshot. No arbitrary path, fake Git history, credential or caller project. Original Source cost/Scope/HMAC/resource/lease/financial gates remain. Model tools allow 128 URLs/skills and the fixed tool byte limit; operator actions retain their larger explicit limits. Acceptance is not completion.', inputSchema: workbench },
    { name: 'scan_list', permission: 'investigation.read', description: 'Find saved service-owned project tasks after reopening the page. Bounded safe projections and a project-bound cursor; no foreign task adoption.', inputSchema: { type: 'object', properties: {
      limit: { type: 'integer', minimum: 1, maximum: 20 }, cursor: { type: 'string', minLength: 1, maxLength: 2048 },
    }, additionalProperties: false } },
    { name: 'scan_draft_create', permission: 'investigation.draft.write', description: 'Create a new original Web-engine draft for explicit HTTP(S) URLs in this host-bound project. No target or provider request starts. The immutable creation receipt is historical; scan_status reads live state.', inputSchema: { type: 'object', properties: {
      title: { type: 'string', minLength: 1, maxLength: 120 },
      targets: { type: 'array', minItems: 1, maxItems: 32, items: { type: 'string', minLength: 1, maxLength: 2048 } },
      scanMode: { type: 'string', enum: ['quick', 'standard', 'deep'] },
    }, required: ['title', 'targets'], additionalProperties: false } },
    ...[
      ['scan_draft_read', 'investigation.read', 'Read a project-owned original task. No private paths, credentials or raw provider material.'],
      ['scan_status', 'investigation.read', 'Read current original scan status and attempt identity; a past accepted receipt is not completion.'],
      ['scan_start', 'investigation.execute', 'Admit a service-owned new Web task through the original model, authorization, lease, budget and financial gates. Requires operator configuration. An accepted startup is not a completed scan.'],
      ['scan_pause', 'investigation.control', 'Request the original pause protocol. Pausing remains pausing until physical owners actually exit; target effects and fees are retained.'],
      ['scan_resume', 'investigation.execute', 'Use original continuation gates for this service birth. Paid or indeterminate history, another executable/store, retired backends and unsupported continuation remain refused. Never reset allowances or retry uncertain operations.'],
      ['scan_retry', 'investigation.execute', 'Use the original retry admission for an eligible task, with unchanged financial and ownership gates. Does not bypass paid-history refusal.'],
      ['scan_draft_cancel', 'investigation.control', 'Delete only an original unconfirmed draft. Preserve its immutable historical creation/action receipts. Running tasks must use scan_pause.'],
    ].map(([name, permission, description]) => ({ name, permission, description, inputSchema: task })),
  ];
  return {
    schemaVersion: 2, id: 'org.oviraptor.scanning', version: '2.0.0', hostApi: '^2.0.0',
    runtime: { kind: 'native', platform, arch }, entry: `bin/oviraptor-scan-service${platform === 'win32' ? '.exe' : ''}`, dataSchemaVersion: 2,
    permissions: [
      ['ui.handoff', 'Allow operator pages to send or receive current-project handoffs for explicit review. No model tool, draft, execution or authorization is granted.'],
      ['investigation.draft.write', 'Create original URL drafts; no execution is implied.'],
      ['investigation.read', 'Read project-owned safe task state.'],
      ['investigation.execute', 'Explicitly admit original scanning/model work under existing engine gates.'],
      ['investigation.control', 'Request original pause or delete an unconfirmed draft.'],
      ['source.import', 'Operator-only project file snapshot import, bounded and immutable; grants no scan execution and is absent from model tools.'],
      ['configuration.read', 'Read application configuration readiness without disclosing credentials.'],
      ['configuration.write', 'Operator-only application model configuration; unavailable to model tools.'],
    ].map(([id, description]) => ({ id, description })),
    tools,
    actions: [
      ...tools.map(tool => ({ ...tool, scope: 'project', ...(tool.name === 'scan_draft_create' ? { inputSchema: { ...tool.inputSchema, properties: { ...tool.inputSchema.properties, targets: { ...tool.inputSchema.properties.targets, maxItems: 200 } } } } : tool.name === 'scan_workbench_start' ? { inputSchema: { ...workbench, properties: { ...workbench.properties, urls: { ...workbench.properties.urls, maxItems: 200 }, skillIds: { ...workbench.properties.skillIds, maxItems: 16384 } } } } : {}) })),
      { name: 'source_import', scope: 'project', permission: 'source.import', description: 'Import explicit operator-selected file bytes into a private immutable snapshot reference. Maximum 128 files, 128 KiB/file, 512 KiB decoded total; whole input must fit the fixed action budget. No arbitrary filesystem path, implicit CWD, Git history or execution permission.', inputSchema: { type: 'object', properties: {
        label: { type: 'string', minLength: 1, maxLength: 128 }, expectedTreeSha256: { type: 'string', minLength: 64, maxLength: 64 },
        files: { type: 'array', minItems: 1, maxItems: 128, items: { type: 'object', properties: { path: { type: 'string', minLength: 1, maxLength: 512 }, contentBase64: { type: 'string', maxLength: 174764 }, sha256: { type: 'string', minLength: 64, maxLength: 64 } }, required: ['path', 'contentBase64', 'sha256'], additionalProperties: false } },
      }, required: ['label', 'expectedTreeSha256', 'files'], additionalProperties: false } },
      { name: 'configuration_status', permission: 'configuration.read', scope: 'application', description: 'Read whether the private original model profile resolves, without making a provider request.', inputSchema: empty },
      { name: 'configure_model', permission: 'configuration.write', scope: 'application', description: 'Configure this application through the original profile persistence and single model resolver. Credentials remain in the application data, never in model tool arguments or results.', inputSchema: { type: 'object', properties: {
        llm: { type: 'string', minLength: 1, maxLength: 200 }, deployment: { type: 'string', enum: ['cloud', 'local'] },
        apiBase: { type: 'string', minLength: 1, maxLength: 2048 }, apiKey: { type: 'string', maxLength: 8192 },
      }, required: ['llm', 'deployment', 'apiBase', 'apiKey'], additionalProperties: false } },
    ],
    pages: [{ id: 'home', title: 'Scanning', path: 'native.html' }], lifecycle: { start: true, stop: true },
  };
}
