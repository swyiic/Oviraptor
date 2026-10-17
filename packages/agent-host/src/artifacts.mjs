import { createHash, randomBytes } from 'node:crypto';
import { hostError, positiveId } from './store.mjs';

const CHUNK_BYTES = 196608;
const MAX_DOWNLOAD_BYTES = 2 * 1024 * 1024 * 1024;
const chunkKeys = ['artifactId', 'offset', 'bytesBase64', 'nextOffset', 'totalBytes', 'sha256', 'done'];
const closed = (value, keys) => value && typeof value === 'object' && !Array.isArray(value) &&
  Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));

/** Generic native artifact delivery. The Host never opens an application's files. */
export class ArtifactDownloads {
  constructor(apps, { now = Date.now, ttlMs = 60000, maxBytes = MAX_DOWNLOAD_BYTES } = {}) {
    this.apps = apps; this.now = now; this.ttlMs = ttlMs; this.maxBytes = maxBytes; this.tickets = new Map(); this.pendingCreates = 0; this.activeStreams = new Set();
  }
  #scope(ticket) {
    const app = this.apps.store.application(ticket.appId), grant = this.apps.store.grant(ticket.appId, ticket.projectId);
    this.apps.store.project(ticket.projectId);
    const action = app.manifest.schemaVersion === 2 && app.manifest.actions.find(item => item.name === ticket.actionName);
    if (this.closed || this.apps.closing || this.apps.changing?.has(ticket.appId) || this.apps.workers?.get(ticket.appId)?.stopped ||
        !app.enabled || app.revision !== ticket.appRevision || app.version !== ticket.version || app.digest !== ticket.digest ||
        grant.revision !== ticket.grantRevision || !action || action.delivery !== 'artifact-read' || action.scope !== 'project' ||
        !grant.permissions.includes(action.permission) || !app.manifest.pages.some(page => page.id === ticket.pageId)) throw hostError('host_artifact_authority_changed', 409);
  }
  async #read(ticket, offset) {
    this.#scope(ticket);
    const value = await this.apps.callAction(ticket.appId, ticket.actionName, ticket.projectId,
      { artifactId: ticket.artifactId, offset, maxBytes: CHUNK_BYTES }, {
        expectedAppRevision: ticket.appRevision, expectedGrantRevision: ticket.grantRevision, pageId: ticket.pageId,
        operationId: `${ticket.token}:${offset}`, applicationScopeAccepted: false,
      });
    this.#scope(ticket);
    if (!closed(value, chunkKeys) || value.artifactId !== ticket.artifactId || value.offset !== offset ||
        !Number.isSafeInteger(value.totalBytes) || value.totalBytes < 0 || value.totalBytes > this.maxBytes ||
        !Number.isSafeInteger(value.nextOffset) || typeof value.bytesBase64 !== 'string' || value.bytesBase64.length > CHUNK_BYTES / 3 * 4 ||
        typeof value.sha256 !== 'string' || !/^[0-9a-f]{64}$/u.test(value.sha256) || typeof value.done !== 'boolean') throw hostError('host_artifact_chunk_invalid');
    const bytes = Buffer.from(value.bytesBase64, 'base64');
    if (bytes.toString('base64') !== value.bytesBase64 || bytes.length > CHUNK_BYTES || value.nextOffset !== offset + bytes.length ||
        value.nextOffset > value.totalBytes || value.done !== (value.nextOffset === value.totalBytes) || !value.done && bytes.length === 0 ||
        ticket.totalBytes !== undefined && (ticket.totalBytes !== value.totalBytes || ticket.sha256 !== value.sha256)) throw hostError('host_artifact_chunk_invalid');
    ticket.totalBytes = value.totalBytes; ticket.sha256 = value.sha256;
    return { bytes, nextOffset: value.nextOffset, done: value.done };
  }
  async create(appId, input) {
    if (!closed(input, ['projectId', 'pageId', 'version', 'expectedAppRevision', 'expectedGrantRevision', 'actionName', 'artifactId', 'filename']) ||
        typeof input.artifactId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/u.test(input.artifactId) ||
        typeof input.filename !== 'string' || !input.filename || input.filename.length > 128 || /[\\/\u0000-\u001f\u007f]/u.test(input.filename)) throw hostError('host_artifact_request_invalid');
    for (const [token, entry] of this.tickets) if (entry.expires <= this.now()) this.tickets.delete(token);
    if (this.closed || this.tickets.size + this.pendingCreates + this.activeStreams.size >= 32) throw hostError('host_artifact_downloads_busy', 409);
    const app = this.apps.store.application(appId);
    const ticket = { token: randomBytes(32).toString('hex'), appId, projectId: positiveId(input.projectId), pageId: input.pageId,
      version: input.version, appRevision: input.expectedAppRevision, grantRevision: input.expectedGrantRevision,
      actionName: input.actionName, artifactId: input.artifactId, filename: input.filename, digest: app.digest, expires: this.now() + this.ttlMs };
    this.pendingCreates++;
    try {
      ticket.first = await this.#read(ticket, 0);
      ticket.expires = this.now() + this.ttlMs;
      this.tickets.set(ticket.token, ticket);
      return { url: `/downloads/${ticket.token}`, filename: ticket.filename, totalBytes: ticket.totalBytes, sha256: ticket.sha256 };
    } finally { this.pendingCreates--; }
  }
  async stream(token, response) {
    const ticket = this.tickets.get(token); this.tickets.delete(token);
    if (!ticket || ticket.expires <= this.now()) throw hostError('host_artifact_ticket_unavailable', 404);
    this.activeStreams.add(response);
    try {
    this.#scope(ticket);
    response.setHeader('Content-Type', 'application/octet-stream');
    response.setHeader('Content-Disposition', `attachment; filename="download.bin"; filename*=UTF-8''${encodeURIComponent(ticket.filename).replace(/['()*]/gu, c => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)}`);
    response.setHeader('Content-Length', String(ticket.totalBytes));
    response.setHeader('X-Content-SHA256', ticket.sha256);
    const hash = createHash('sha256'); let chunk = ticket.first;
    while (true) {
      this.#scope(ticket);
      if (response.destroyed) throw hostError('host_artifact_download_closed');
      hash.update(chunk.bytes);
      // Withhold the final chunk on a digest mismatch so the HTTP download is
      // incomplete, rather than reporting a corrupt complete attachment.
      if (chunk.done && hash.digest('hex') !== ticket.sha256) throw hostError('host_artifact_digest_mismatch');
      if (!response.write(chunk.bytes)) await new Promise((resolve, reject) => {
        const cleanup = () => { response.off('drain', drain); response.off('close', close); response.off('error', error); };
        const drain = () => { cleanup(); resolve(); };
        const close = () => { cleanup(); reject(hostError('host_artifact_download_closed')); };
        const error = () => { cleanup(); reject(hostError('host_artifact_download_closed')); };
        response.once('drain', drain); response.once('close', close); response.once('error', error);
      });
      this.#scope(ticket);
      if (chunk.done) { response.end(); return; }
      chunk = await this.#read(ticket, chunk.nextOffset);
    }
    } finally { this.activeStreams.delete(response); }
  }
  close() { this.closed = true; this.tickets.clear(); for (const response of this.activeStreams) response.destroy(); }
}
