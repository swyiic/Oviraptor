import { mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { hostError } from './store.mjs';

const EXCERPT_BYTES = 480;
const MAX_IMPORT = 32;

function clipUtf8(text, maxBytes) {
  const bytes = Buffer.from(text);
  let end = Math.min(maxBytes, bytes.length);
  while (end > 0 && end < bytes.length && (bytes[end] & 0xc0) === 0x80) end--;
  return bytes.subarray(0, end).toString('utf8');
}

export function listPairedSkillEntries(manifest) {
  if (!Array.isArray(manifest?.paired_skills)) return [];
  return manifest.paired_skills.map(item => ({
    name: item.name,
    source: item.source,
    required: Boolean(item.required),
  }));
}

function libraryByName(library) {
  const map = new Map();
  if (!library?.listSkills) return map;
  for (const skill of library.listSkills()) {
    if (skill.enabled === false) continue;
    if (!map.has(skill.name)) map.set(skill.name, skill);
  }
  return map;
}

function libraryAnyByName(library) {
  const map = new Map();
  if (!library?.listSkills) return map;
  for (const skill of library.listSkills()) {
    if (!map.has(skill.name)) map.set(skill.name, skill);
  }
  return map;
}

async function readPackageReference(appPath, source) {
  const absolute = join(appPath, source);
  const text = await readFile(absolute, 'utf8');
  if (text.includes('\0') || Buffer.byteLength(text) > 1_048_576) throw hostError('host_paired_skill_source_invalid');
  return text;
}

/**
 * Build a preview of paired_skills for an installed application.
 * Reference-only: never grants tools or execution.
 */
export async function buildPairedSkillsPreview({ app, library }) {
  if (!app?.manifest) throw hostError('host_app_unavailable', 404);
  const entries = listPairedSkillEntries(app.manifest);
  const byName = libraryAnyByName(library);
  const skills = [];
  for (const entry of entries) {
    const match = byName.get(entry.name);
    let excerpt = null;
    let digest = null;
    let status = 'missing';
    let importable = false;
    if (match) status = match.enabled ? 'in_library' : 'disabled_in_library';
    if (entry.source === 'bundled') {
      if (status === 'missing') {
        status = 'bundled_reference';
        importable = false; // Host cannot invent bundled Skill bytes; import bundled library separately or use package-relative sources.
      } else {
        importable = false;
      }
    } else {
      try {
        const text = await readPackageReference(app.path, entry.source);
        excerpt = clipUtf8(text, EXCERPT_BYTES);
        digest = createHash('sha256').update(text).digest('hex');
        if (status === 'missing') {
          status = 'package_reference';
          importable = true;
        } else if (status === 'disabled_in_library') {
          importable = false;
        } else {
          importable = false;
        }
      } catch {
        status = 'source_unavailable';
        importable = false;
      }
    }
    skills.push({
      name: entry.name,
      source: entry.source,
      required: entry.required,
      status,
      skillId: match?.id ?? null,
      description: match?.description ?? null,
      excerpt,
      digest,
      importable,
      inSkillsReadScope: Boolean(match?.enabled),
    });
  }
  return {
    appId: app.id,
    note: 'Paired Skills are untrusted reference guidance only. Importing them grants no application tools, project access, or execution authority. Uninstalling the application does not delete Skills.',
    skills,
  };
}

async function stageSkillPackage({ name, markdown, stagingRoot }) {
  const dir = join(stagingRoot, `paired-skill-${randomUUID()}`);
  await mkdir(dir, { recursive: true, mode: 0o700 });
  const stripped = markdown.replace(/^\uFEFF?---[\s\S]*?\n---\r?\n?/u, '');
  const description = `Paired skill reference imported from an application package (read-only; no execution).`;
  const body = `---\nname: ${name}\ndescription: ${description}\n---\n${stripped}`;
  await writeFile(join(dir, 'SKILL.md'), body, { mode: 0o600 });
  return dir;
}

/**
 * Import selected package-relative paired skills into the Skill library, then
 * optionally add their IDs to a conversation's skills_read selection.
 * Uninstalling the application never deletes imported Skills.
 */
export async function importPairedSkills({
  app,
  library,
  skillAccess,
  names,
  conversationId = null,
  includeInSkillsRead = false,
  runtimeIdentity,
  stagingRoot = tmpdir(),
} = {}) {
  if (!app?.manifest) throw hostError('host_app_unavailable', 404);
  if (!library?.previewDirectory || !library?.importDirectory) throw hostError('host_skill_library_unavailable', 503);
  const entries = listPairedSkillEntries(app.manifest);
  if (!entries.length) {
    return { appId: app.id, imported: [], alreadyPresent: [], skipped: [], conversationSelection: null, note: 'No paired_skills declared.' };
  }
  let selectedNames;
  if (names === undefined || names === null) selectedNames = entries.map(item => item.name);
  else {
    if (!Array.isArray(names) || names.length > MAX_IMPORT || names.some(name => typeof name !== 'string' || !name || name.length > 160)) {
      throw hostError('host_paired_skill_names_invalid');
    }
    selectedNames = [...new Set(names)];
  }
  const allowed = new Map(entries.map(item => [item.name, item]));
  for (const name of selectedNames) if (!allowed.has(name)) throw hostError('host_paired_skill_unknown', 404);

  const imported = [];
  const alreadyPresent = [];
  const skipped = [];
  const byNameBefore = libraryAnyByName(library);

  for (const name of selectedNames) {
    const entry = allowed.get(name);
    const existing = byNameBefore.get(name);
    if (existing?.enabled) {
      alreadyPresent.push({ name, skillId: existing.id, source: entry.source });
      continue;
    }
    if (entry.source === 'bundled') {
      skipped.push({
        name,
        source: entry.source,
        reason: existing ? 'disabled_in_library' : 'bundled_bytes_unavailable',
        hint: existing
          ? 'Re-enable the existing Skill in the library; application install does not overwrite Skills.'
          : 'Install the host bundled Skill library (or a package that provides this Skill name), then retry include.',
      });
      continue;
    }
    let staged;
    try {
      const markdown = await readPackageReference(app.path, entry.source);
      staged = await stageSkillPackage({ name, markdown, stagingRoot });
      const preview = await library.previewDirectory(staged);
      await library.importDirectory(staged, preview.digest);
      const after = libraryAnyByName(library).get(name);
      if (!after) throw hostError('host_paired_skill_import_unverified');
      imported.push({ name, skillId: after.id, source: entry.source, digest: preview.digest });
      byNameBefore.set(name, after);
    } catch (error) {
      skipped.push({ name, source: entry.source, reason: error.code ?? 'import_failed', hint: error.message || String(error) });
    } finally {
      if (staged) await rm(staged, { recursive: true, force: true });
    }
  }

  let conversationSelection = null;
  if (includeInSkillsRead) {
    if (conversationId === null || conversationId === undefined) throw hostError('host_paired_skill_conversation_required');
    if (!skillAccess?.get || !skillAccess?.set) throw hostError('host_skill_access_unavailable', 503);
    const identity = typeof runtimeIdentity === 'function' ? runtimeIdentity() : runtimeIdentity;
    if (typeof identity !== 'string' || !identity) throw hostError('host_skill_access_runtime_invalid');
    const current = skillAccess.get(conversationId);
    const readableIds = new Set(current.selectedSkillIds);
    const byName = libraryByName(library);
    for (const name of selectedNames) {
      const skill = byName.get(name);
      if (skill) readableIds.add(skill.id);
    }
    const nextIds = [...readableIds];
    conversationSelection = skillAccess.set(conversationId, {
      selectedSkillIds: nextIds,
      expectedRevision: current.revision,
      disclosureAccepted: true,
      expectedRuntimeIdentity: identity,
      expectedSkillRevision: current.skillRevision,
    });
  }

  return {
    appId: app.id,
    imported,
    alreadyPresent,
    skipped,
    conversationSelection,
    note: 'Imported Skills remain after application uninstall. They grant no tools or execution authority.',
  };
}
