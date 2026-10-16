// Native execution is a versioned package. The Node v1 draft package remains
// separate and is never allowed to spawn a native executable implicitly.
export function createNativeManifest({ platform = process.platform, arch = process.arch } = {}) {
  const task = { type: 'object', properties: { taskId: { type: 'string', minLength: 1, maxLength: 128 } }, required: ['taskId'], additionalProperties: false };
  const empty = { type: 'object', properties: {}, additionalProperties: false };
  const tools = [
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
      ['investigation.draft.write', 'Create original URL drafts; no execution is implied.'],
      ['investigation.read', 'Read project-owned safe task state.'],
      ['investigation.execute', 'Explicitly admit original scanning/model work under existing engine gates.'],
      ['investigation.control', 'Request original pause or delete an unconfirmed draft.'],
      ['configuration.read', 'Read application configuration readiness without disclosing credentials.'],
      ['configuration.write', 'Operator-only application model configuration; unavailable to model tools.'],
    ].map(([id, description]) => ({ id, description })),
    tools,
    actions: [
      ...tools.map(tool => ({ ...tool, scope: 'project' })),
      { name: 'configuration_status', permission: 'configuration.read', scope: 'application', description: 'Read whether the private original model profile resolves, without making a provider request.', inputSchema: empty },
      { name: 'configure_model', permission: 'configuration.write', scope: 'application', description: 'Configure this application through the original profile persistence and single model resolver. Credentials remain in the application data, never in model tool arguments or results.', inputSchema: { type: 'object', properties: {
        llm: { type: 'string', minLength: 1, maxLength: 200 }, deployment: { type: 'string', enum: ['cloud', 'local'] },
        apiBase: { type: 'string', minLength: 1, maxLength: 2048 }, apiKey: { type: 'string', maxLength: 8192 },
      }, required: ['llm', 'deployment', 'apiBase', 'apiKey'], additionalProperties: false } },
    ],
    pages: [{ id: 'home', title: 'Scanning', path: 'native.html' }], lifecycle: { start: true, stop: true },
  };
}
