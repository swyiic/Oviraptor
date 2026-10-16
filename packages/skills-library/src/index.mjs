import { DatabaseSync } from 'node:sqlite';
import { mkdirSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { LIMITS, allowedResource, bundledPackage, checkedPath, error, hash, localPackage, notices, privateResource, resourceKind, sensitiveText, utf8 } from './import.mjs';

export { LIMITS } from './import.mjs';
const stamp = () => new Date().toISOString();
const positiveId = id => { const value = Number(id); if (!Number.isSafeInteger(value) || value < 1) throw error('skill_package_skill_unavailable'); return value; };

/** Declarative guidance storage only: no model, executable loader or host grants. */
export class SkillLibrary {
  constructor({ dataDir } = {}) {
    if (typeof dataDir !== 'string' || !dataDir || dataDir.includes('\0')) throw error('skill_package_data_dir_invalid');
    this.dataDir = resolve(dataDir); mkdirSync(this.dataDir, { recursive: true, mode: 0o700 });
    this.db = new DatabaseSync(join(this.dataDir, 'skills.sqlite'));
    this.db.exec(`PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
      CREATE TABLE IF NOT EXISTS skill_packages(
        id TEXT PRIMARY KEY,package_key TEXT NOT NULL,name TEXT NOT NULL,source TEXT NOT NULL,version TEXT NOT NULL,license TEXT NOT NULL,
        builtin INTEGER NOT NULL CHECK(builtin IN(0,1)),digest TEXT NOT NULL,file_count INTEGER NOT NULL CHECK(file_count BETWEEN 1 AND 512),
        total_bytes INTEGER NOT NULL CHECK(total_bytes BETWEEN 1 AND 16777216),created_at TEXT NOT NULL,
        installed INTEGER NOT NULL DEFAULT 1 CHECK(installed IN(0,1)),revision INTEGER NOT NULL DEFAULT 1 CHECK(typeof(revision)='integer' AND revision BETWEEN 1 AND 9007199254740991));
      CREATE TABLE IF NOT EXISTS skill_entries(
        id INTEGER PRIMARY KEY AUTOINCREMENT,package_id TEXT NOT NULL REFERENCES skill_packages(id),name TEXT NOT NULL,description TEXT NOT NULL,
        entry_path TEXT NOT NULL,instructions TEXT NOT NULL,enabled INTEGER NOT NULL CHECK(enabled IN(0,1)),
        generation INTEGER NOT NULL DEFAULT 1 CHECK(typeof(generation)='integer' AND generation BETWEEN 1 AND 9007199254740991),UNIQUE(package_id,entry_path));
      CREATE TABLE IF NOT EXISTS skill_files(
        package_id TEXT NOT NULL REFERENCES skill_packages(id),path TEXT NOT NULL,content BLOB NOT NULL,
        size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 0 AND 1048576),sha256 TEXT NOT NULL,text INTEGER NOT NULL CHECK(text IN(0,1)),
        kind TEXT NOT NULL CHECK(kind IN('instructions','reference','script','resource')),PRIMARY KEY(package_id,path));`);
    // Persistent generations record actual changes even when content is later
    // restored before the next read. Identical writes do not revoke consent.
    const resourceScope = alias => `(package_id=${alias}.package_id AND (
      (substr(${alias}.path,1,length(entry_path)-8)=substr(entry_path,1,length(entry_path)-8)
        AND (length(entry_path)>8 OR substr(${alias}.path,1,7)<>'skills/'))
      OR ${alias}.path IN('LICENSE','LICENSE.md','LICENSE.txt','NOTICE')))`;
    this.db.exec(`
      CREATE TRIGGER IF NOT EXISTS skill_generation_entry_change
      AFTER UPDATE OF name,description,instructions,enabled ON skill_entries
      WHEN NEW.name IS NOT OLD.name OR NEW.description IS NOT OLD.description OR NEW.instructions IS NOT OLD.instructions OR NEW.enabled IS NOT OLD.enabled
      BEGIN
        UPDATE skill_entries SET generation=generation+1 WHERE id=NEW.id;
        UPDATE skill_packages SET revision=revision+1 WHERE id=NEW.package_id
          AND (NEW.name IS NOT OLD.name OR NEW.description IS NOT OLD.description OR NEW.instructions IS NOT OLD.instructions);
      END;
      CREATE TRIGGER IF NOT EXISTS skill_generation_package_change
      AFTER UPDATE OF package_key,name,source,version,license,builtin,digest,file_count,total_bytes ON skill_packages
      WHEN NEW.package_key IS NOT OLD.package_key OR NEW.name IS NOT OLD.name OR NEW.source IS NOT OLD.source OR NEW.version IS NOT OLD.version
        OR NEW.license IS NOT OLD.license OR NEW.builtin IS NOT OLD.builtin OR NEW.digest IS NOT OLD.digest OR NEW.file_count IS NOT OLD.file_count OR NEW.total_bytes IS NOT OLD.total_bytes
      BEGIN
        UPDATE skill_entries SET generation=generation+1 WHERE package_id=NEW.id;
        UPDATE skill_packages SET revision=revision+1 WHERE id=NEW.id;
      END;
      CREATE TRIGGER IF NOT EXISTS skill_generation_file_insert AFTER INSERT ON skill_files
      BEGIN
        UPDATE skill_entries SET generation=generation+1 WHERE ${resourceScope('NEW')};
        UPDATE skill_packages SET revision=revision+1 WHERE id=NEW.package_id
          AND EXISTS(SELECT 1 FROM skill_entries WHERE package_id=NEW.package_id);
      END;
      CREATE TRIGGER IF NOT EXISTS skill_generation_file_delete AFTER DELETE ON skill_files
      BEGIN
        UPDATE skill_entries SET generation=generation+1 WHERE ${resourceScope('OLD')};
        UPDATE skill_packages SET revision=revision+1 WHERE id=OLD.package_id;
      END;
      CREATE TRIGGER IF NOT EXISTS skill_generation_file_change
      AFTER UPDATE OF package_id,path,content,size_bytes,sha256,text,kind ON skill_files
      WHEN NEW.package_id IS NOT OLD.package_id OR NEW.path IS NOT OLD.path OR NEW.content IS NOT OLD.content OR NEW.size_bytes IS NOT OLD.size_bytes
        OR NEW.sha256 IS NOT OLD.sha256 OR NEW.text IS NOT OLD.text OR NEW.kind IS NOT OLD.kind
      BEGIN
        UPDATE skill_entries SET generation=generation+1 WHERE ${resourceScope('OLD')} OR ${resourceScope('NEW')};
        UPDATE skill_packages SET revision=revision+1 WHERE id=OLD.package_id OR id=NEW.package_id;
      END;`);
  }
  close() { this.db.close(); }
  async previewDirectory(path) {
    const pkg = await localPackage(path);
    return { digest: pkg.digest, name: pkg.name, skillCount: pkg.skills.length, fileCount: pkg.files.length, totalBytes: pkg.totalBytes,
      skills: pkg.skills.map(({ name, description, entryPath }) => ({ name, description, entryPath })), missingCapabilities: notices(),
      warnings: ['Existing skill names are never overwritten. Importing does not grant these skills to any conversation.'] };
  }
  async importDirectory(path, expectedDigest) {
    if (typeof expectedDigest !== 'string' || !/^[0-9a-f]{64}$/u.test(expectedDigest)) throw error('skill_package_preview_changed');
    const pkg = await localPackage(path);
    if (pkg.digest !== expectedDigest) throw error('skill_package_preview_changed');
    return this.#install(pkg);
  }
  async installBundled(manifestPath) { return this.#install(await bundledPackage(manifestPath)); }
  #transaction(action) {
    this.db.exec('BEGIN IMMEDIATE');
    try { const result = action(); this.db.exec('COMMIT'); return result; }
    catch (cause) { this.db.exec('ROLLBACK'); if (cause.code?.startsWith('skill_package_')) throw cause; throw error('skill_package_storage_failed'); }
  }
  #package(id, installed = true) {
    const pkg = this.db.prepare('SELECT * FROM skill_packages WHERE id=?').get(String(id));
    if (!pkg || installed && !pkg.installed) throw error('skill_package_unavailable', 404);
    return pkg;
  }
  #skill(id) {
    const row = this.db.prepare('SELECT s.* FROM skill_entries s JOIN skill_packages p ON p.id=s.package_id WHERE s.id=? AND p.installed=1').get(positiveId(id));
    if (!row) throw error('skill_package_skill_unavailable', 404);
    return row;
  }
  #viewSkill(row) {
    return { id: row.id, packageId: row.package_id, name: row.name, description: row.description, entryPath: row.entry_path, enabled: Boolean(row.enabled), generation: row.generation };
  }
  #info(id) {
    const pkg = this.#package(id), skills = this.db.prepare('SELECT * FROM skill_entries WHERE package_id=? ORDER BY name,id').all(pkg.id).map(row => this.#viewSkill(row));
    return { id: pkg.id, packageKey: pkg.package_key, name: pkg.name, source: pkg.source, version: pkg.version, license: pkg.license, builtin: Boolean(pkg.builtin),
      digest: pkg.digest, revision: pkg.revision, skillCount: skills.length, fileCount: pkg.file_count, totalBytes: pkg.total_bytes, createdAt: pkg.created_at, skills, missingCapabilities: notices() };
  }
  listPackages() { return this.db.prepare('SELECT id FROM skill_packages WHERE installed=1 ORDER BY builtin DESC,name,id').all().map(row => this.#info(row.id)); }
  listSkills() { return this.db.prepare('SELECT s.* FROM skill_entries s JOIN skill_packages p ON p.id=s.package_id WHERE p.installed=1 ORDER BY s.name,s.id').all().map(row => this.#viewSkill(row)); }
  #install(pkg) {
    return this.#transaction(() => {
      const previous = this.db.prepare('SELECT * FROM skill_packages WHERE id=?').get(pkg.id);
      if (previous && previous.digest !== pkg.digest) throw error('skill_package_version_conflict', 409);
      if (previous?.installed) { this.#verifyRetained(pkg); return this.#info(pkg.id); }
      const names = new Set(this.db.prepare('SELECT s.name FROM skill_entries s JOIN skill_packages p ON p.id=s.package_id WHERE p.installed=1').all().map(row => row.name.toLowerCase()));
      if (previous) {
        // The bytes and IDs survive uninstall. Verify the retained snapshot
        // before reactivation; reactivation never recreates a conversation grant.
        this.#verifyRetained(pkg);
        const oldSkills = this.db.prepare('SELECT * FROM skill_entries WHERE package_id=?').all(pkg.id);
        for (const skill of oldSkills) if (names.has(skill.name.toLowerCase())) throw error(`skill_package_name_conflict:${skill.name}`, 409);
        this.db.prepare('UPDATE skill_entries SET enabled=1,generation=generation+CASE WHEN enabled=1 THEN 1 ELSE 0 END WHERE package_id=?').run(pkg.id);
        this.db.prepare('UPDATE skill_packages SET installed=1,revision=revision+1 WHERE id=?').run(pkg.id);
        return this.#info(pkg.id);
      }
      const reserved = new Set(pkg.skills.map(skill => skill.name.toLowerCase()));
      for (const skill of pkg.skills) {
        const original = skill.name; let name = original, ordinal = 0;
        while (names.has(name.toLowerCase())) {
          if (!pkg.builtin) throw error(`skill_package_name_conflict:${original}`, 409);
          do {
            const suffix = ++ordinal === 1 ? ' (Every Compound)' : ` (Every Compound ${ordinal})`;
            name = [...original].slice(0, 80 - [...suffix].length).join('') + suffix;
          } while (reserved.has(name.toLowerCase()));
        }
        names.add(name.toLowerCase()); reserved.add(name.toLowerCase()); skill.name = name;
      }
      this.db.prepare('INSERT INTO skill_packages(id,package_key,name,source,version,license,builtin,digest,file_count,total_bytes,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)')
        .run(pkg.id, pkg.packageKey, pkg.name, pkg.source, pkg.version, pkg.license, Number(pkg.builtin), pkg.digest, pkg.files.length, pkg.totalBytes, stamp());
      const fileInsert = this.db.prepare('INSERT INTO skill_files(package_id,path,content,size_bytes,sha256,text,kind) VALUES(?,?,?,?,?,?,?)');
      for (const file of pkg.files) fileInsert.run(pkg.id, file.path, file.bytes, file.bytes.length, file.sha256, Number(utf8(file.bytes) !== null && !file.bytes.includes(0)), resourceKind(file.path));
      const skillInsert = this.db.prepare('INSERT INTO skill_entries(package_id,name,description,entry_path,instructions,enabled) VALUES(?,?,?,?,?,1)');
      for (const skill of pkg.skills) skillInsert.run(pkg.id, skill.name, skill.description, skill.entryPath, skill.instructions);
      return this.#info(pkg.id);
    });
  }
  #verifyRetained(pkg) {
    const stored = this.#package(pkg.id, false);
    for (const [column, expected] of Object.entries({ package_key: pkg.packageKey, name: pkg.name, source: pkg.source, version: pkg.version,
      license: pkg.license, builtin: Number(pkg.builtin), digest: pkg.digest, file_count: pkg.files.length, total_bytes: pkg.totalBytes })) {
      if (stored[column] !== expected) throw error('skill_package_resource_integrity_failed');
    }
    const rows = this.db.prepare('SELECT * FROM skill_files WHERE package_id=? ORDER BY path').all(pkg.id);
    if (rows.length !== pkg.files.length) throw error('skill_package_resource_integrity_failed');
    const expected = new Map(pkg.files.map(file => [file.path, file]));
    for (const row of rows) {
      const file = expected.get(row.path), bytes = Buffer.from(row.content);
      if (!file || row.size_bytes !== file.bytes.length || row.sha256 !== file.sha256 || hash(bytes) !== file.sha256
        || row.kind !== resourceKind(file.path) || Boolean(row.text) !== (utf8(bytes) !== null && !bytes.includes(0))) throw error('skill_package_resource_integrity_failed');
    }
    const entries = this.db.prepare('SELECT * FROM skill_entries WHERE package_id=?').all(pkg.id);
    if (entries.length !== pkg.skills.length || entries.some(row => pkg.skills.find(skill => skill.entryPath === row.entry_path)?.instructions !== row.instructions)) throw error('skill_package_resource_integrity_failed');
  }
  setEnabled(id, enabled) {
    if (typeof enabled !== 'boolean') throw error('skill_package_enabled_invalid');
    return this.#transaction(() => {
      const skill = this.#skill(id);
      if (Boolean(skill.enabled) !== enabled) {
        this.db.prepare('UPDATE skill_entries SET enabled=? WHERE id=?').run(Number(enabled), skill.id);
        this.db.prepare('UPDATE skill_packages SET revision=revision+1 WHERE id=?').run(skill.package_id);
      }
      return this.#viewSkill(this.#skill(id));
    });
  }
  uninstallPackage(id, { expectedRevision } = {}) {
    return this.#transaction(() => {
      const pkg = this.#package(id);
      if (!Number.isSafeInteger(expectedRevision) || expectedRevision !== pkg.revision) throw error('skill_package_revision_conflict', 409);
      this.db.prepare('UPDATE skill_entries SET enabled=0,generation=generation+CASE WHEN enabled=0 THEN 1 ELSE 0 END WHERE package_id=?').run(pkg.id);
      this.db.prepare('UPDATE skill_packages SET installed=0,revision=revision+1 WHERE id=?').run(pkg.id);
      return { id: pkg.id, uninstalled: true, dataRetained: true, revision: pkg.revision + 1 };
    });
  }
  listResources(skillId) {
    const skill = this.#skill(skillId), rows = this.#files(skill.package_id);
    return { skillId: skill.id, packageId: skill.package_id, resources: rows.filter(row => allowedResource(skill.entry_path, row.path) && !privateResource(row.path))
      .map(row => ({ path: row.path, sizeBytes: row.size_bytes, sha256: row.sha256, text: Boolean(row.text), kind: row.kind })) };
  }
  #files(packageId) {
    const files = this.db.prepare('SELECT path,size_bytes,sha256,text,kind FROM skill_files WHERE package_id=? ORDER BY path LIMIT 513').all(packageId);
    if (files.length > LIMITS.files) throw error('skill_package_file_limit');
    return files;
  }
  #bytes(packageId, path) {
    const row = this.db.prepare('SELECT content,size_bytes,sha256 FROM skill_files WHERE package_id=? AND path=? AND length(content)<=?').get(packageId, path, LIMITS.fileBytes);
    if (!row) throw error('skill_package_resource_unavailable', 404);
    const bytes = Buffer.from(row.content);
    if (row.size_bytes !== bytes.length || hash(bytes) !== row.sha256) throw error('skill_package_resource_integrity_failed');
    return bytes;
  }
  readResource(skillId, path, offset = 0, limit = 4096) {
    checkedPath(path);
    if (!Number.isSafeInteger(limit) || limit < 1 || limit > LIMITS.resourcePageBytes) throw error('skill_package_page_limit');
    const skill = this.#skill(skillId);
    if (!allowedResource(skill.entry_path, path) || privateResource(path)) throw error('skill_package_resource_denied', 403);
    const bytes = this.#bytes(skill.package_id, path);
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > bytes.length) throw error('skill_package_offset_invalid');
    const text = utf8(bytes);
    if (text === null || bytes.includes(0)) return { skillId: skill.id, path, content: null, encoding: 'binary', offset: 0, nextOffset: null, hasMore: false, totalBytes: bytes.length };
    if (sensitiveText(text)) throw error('skill_package_credential_content_denied', 403);
    const boundary = index => index === bytes.length || (bytes[index] & 0xc0) !== 0x80;
    if (!boundary(offset)) throw error('skill_package_offset_invalid');
    let end = Math.min(offset + limit, bytes.length); while (end > offset && !boundary(end)) end--;
    if (end === offset && offset < bytes.length) throw error('skill_package_page_too_small');
    return { skillId: skill.id, path, content: bytes.subarray(offset, end).toString('utf8'), encoding: 'utf8', offset,
      nextOffset: end < bytes.length ? end : null, hasMore: end < bytes.length, totalBytes: bytes.length };
  }
  fingerprint(skillId) {
    const skill = this.#skill(skillId); if (!skill.enabled) throw error('skill_package_skill_unavailable', 403);
    const pkg = this.#package(skill.package_id), files = []; let total = 0, foundEntry = false;
    for (const row of this.#files(pkg.id)) {
      checkedPath(row.path); if (!allowedResource(skill.entry_path, row.path)) continue;
      const bytes = this.#bytes(pkg.id, row.path); total += bytes.length;
      if (total > LIMITS.totalBytes) throw error('skill_package_byte_limit');
      foundEntry ||= row.path === skill.entry_path;
      files.push({ path: row.path, size: bytes.length, sha256: hash(bytes), text: Boolean(row.text), kind: row.kind });
    }
    if (!foundEntry) throw error('skill_package_resource_integrity_failed');
    return hash(JSON.stringify({ skillId: skill.id, generation: skill.generation, name: skill.name, description: skill.description, instructions: skill.instructions,
      packageId: pkg.id, entry: skill.entry_path, metadata: { packageKey: pkg.package_key, name: pkg.name, source: pkg.source, version: pkg.version,
        license: pkg.license, builtin: Boolean(pkg.builtin), digest: pkg.digest, fileCount: pkg.file_count, totalBytes: pkg.total_bytes }, files }));
  }
}
