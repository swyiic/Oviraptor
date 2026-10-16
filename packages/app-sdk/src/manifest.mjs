import { fail, jsonBytes, keys, text, MAX_TOOL_RESULT_BYTES } from './protocol.mjs';
import { validateInputSchema } from './schema.mjs';

export const MANIFEST_LIMITS = Object.freeze({ manifestBytes: 128 * 1024, files: 256, fileBytes: 8 * 1024 * 1024, packageBytes: 32 * 1024 * 1024, tools: 64, pages: 16, permissions: 64 });
const ID = /^(?=.{3,128}$)[a-z][a-z0-9]*(?:-[a-z0-9]+)*(?:\.[a-z][a-z0-9]*(?:-[a-z0-9]+)*)+$/u;
const NAME = /^[a-z][a-z0-9_]{0,63}$/u;
const PERMISSION = /^[a-z][a-z0-9._-]{0,95}$/u;
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9]\d*|\d*[a-zA-Z-][0-9a-zA-Z-]*))*)?(?:\+[0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*)?$/u;
const invalid = message => fail('app_manifest_invalid', message);
export function validatePackagePath(value) {
  text(value, 240, 'package path', { code: 'app_manifest_invalid' });
  if (!/^[a-zA-Z0-9_.\-/]+$/u.test(value) || value.startsWith('/') || value.endsWith('/') || value.includes('\\') || value.includes('//')) invalid('Package paths must be portable relative paths');
  for (const part of value.split('/')) {
    if (part === '.' || part === '..' || part.endsWith('.') || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(part)) invalid('Package path has a forbidden component');
  }
  return value;
}
export function qualifiedToolName(appId, localName) {
  text(appId, 128, 'app id', { pattern: ID, code: 'app_manifest_invalid' });
  text(localName, 64, 'tool name', { pattern: NAME, code: 'app_manifest_invalid' });
  return `${appId}::${localName}`;
}
function array(value, maximum, where) {
  if (!Array.isArray(value) || value.length > maximum) invalid(`${where} must be a bounded array`);
}
function unique(values, where) { if (new Set(values).size !== values.length) invalid(`${where} contains duplicates`); }
export function validateManifest(value) {
  jsonBytes(value, MANIFEST_LIMITS.manifestBytes, 'app_manifest_invalid');
  keys(value, ['schemaVersion', 'id', 'version', 'hostApi', 'entry', 'dataSchemaVersion', 'pages', 'tools', 'permissions', 'lifecycle', 'files'],
    ['schemaVersion', 'id', 'version', 'hostApi', 'entry', 'dataSchemaVersion', 'pages', 'tools', 'permissions', 'files'], 'manifest', 'app_manifest_invalid');
  if (value.schemaVersion !== 1 || value.hostApi !== '^1.0.0') invalid('Unsupported manifest or host API version');
  text(value.id, 128, 'manifest.id', { pattern: ID, code: 'app_manifest_invalid' });
  text(value.version, 80, 'manifest.version', { pattern: SEMVER, code: 'app_manifest_invalid' });
  if (value.entry !== 'worker.mjs') invalid('Manifest entry must be worker.mjs');
  if (!Number.isSafeInteger(value.dataSchemaVersion) || value.dataSchemaVersion < 1) invalid('dataSchemaVersion must be a positive safe integer');
  array(value.permissions, MANIFEST_LIMITS.permissions, 'permissions');
  for (const item of value.permissions) {
    keys(item, ['id', 'description'], ['id', 'description'], 'permission', 'app_manifest_invalid');
    text(item.id, 96, 'permission.id', { pattern: PERMISSION, code: 'app_manifest_invalid' });
    text(item.description, 1024, 'permission.description', { code: 'app_manifest_invalid' });
  }
  unique(value.permissions.map(item => item.id), 'permissions');
  array(value.tools, MANIFEST_LIMITS.tools, 'tools');
  for (const tool of value.tools) {
    keys(tool, ['name', 'description', 'inputSchema', 'permission'], ['name', 'description', 'inputSchema', 'permission'], 'tool', 'app_manifest_invalid');
    qualifiedToolName(value.id, tool.name);
    text(tool.description, 2048, 'tool.description', { code: 'app_manifest_invalid' });
    if (!value.permissions.some(item => item.id === tool.permission)) invalid('Tool references an undeclared permission');
    validateInputSchema(tool.inputSchema);
    if (tool.inputSchema.type !== 'object') invalid('Tool input schema must be an object');
  }
  unique(value.tools.map(item => item.name), 'tools');
  array(value.pages, MANIFEST_LIMITS.pages, 'pages');
  for (const page of value.pages) {
    keys(page, ['id', 'title', 'path'], ['id', 'title', 'path'], 'page', 'app_manifest_invalid');
    text(page.id, 64, 'page.id', { pattern: NAME, code: 'app_manifest_invalid' });
    text(page.title, 128, 'page.title', { code: 'app_manifest_invalid' });
    validatePackagePath(page.path);
    if (!page.path.endsWith('.html')) invalid('Page paths must end in .html');
  }
  unique(value.pages.map(item => item.id), 'pages');
  if (value.lifecycle !== undefined) {
    keys(value.lifecycle, ['start', 'stop'], [], 'lifecycle', 'app_manifest_invalid');
    for (const flag of Object.values(value.lifecycle)) if (typeof flag !== 'boolean') invalid('Lifecycle flags must be booleans');
  }
  array(value.files, MANIFEST_LIMITS.files, 'files');
  for (const item of value.files) {
    keys(item, ['path', 'sha256'], ['path', 'sha256'], 'file', 'app_manifest_invalid');
    validatePackagePath(item.path);
    if (item.path.toLowerCase() === 'manifest.json') invalid('manifest.json cannot hash itself');
    if (!/^[0-9a-f]{64}$/u.test(item.sha256)) invalid('File sha256 must be lowercase SHA-256');
  }
  const paths = value.files.map(item => item.path);
  unique(paths.map(path => path.toLowerCase()), 'file paths');
  for (const path of paths) if (paths.some(other => other !== path && other.toLowerCase().startsWith(`${path.toLowerCase()}/`))) invalid('A file cannot be a directory');
  for (const path of [value.entry, ...value.pages.map(item => item.path)]) if (!paths.includes(path)) invalid('Every entry and page must be hash listed');
  jsonBytes({ protocolVersion: 1, appId: value.id, version: value.version, dataSchemaVersion: value.dataSchemaVersion, tools: value.tools, pages: value.pages }, MAX_TOOL_RESULT_BYTES, 'app_manifest_invalid');
  return structuredClone(value);
}
