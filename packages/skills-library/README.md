# Declarative Skills library

`SkillLibrary` is a Node 24, dependency-free SQLite service for instruction and reference packages. It does not execute scripts, install dependencies, call a model, register application tools, or grant access to any project, provider, directory or conversation. App installation and Skill import remain separate operations.

```js
import { SkillLibrary } from './src/index.mjs';

const library = new SkillLibrary({ dataDir: '/absolute/private/skills' });
const preview = await library.previewDirectory('/selected/skill-folder');
// Show preview.skills, preview.warnings and missingCapabilities before approval.
const saved = await library.importDirectory('/selected/skill-folder', preview.digest);
const skill = saved.skills[0];
const resources = library.listResources(skill.id);
const page = library.readResource(skill.id, skill.entryPath, 0, 4096);
// Continue with page.nextOffset, a UTF-8 byte offset, while page.hasMore.
library.close();
```

The library owns only `skills.sqlite` beneath `dataDir`, with `skill_packages`, `skill_entries`, `skill_files` and SQLite's monotonic ID sequence. It never opens a legacy application database or imports asset, scanning or conversation tables. The directory is not a source package and the database contains the copied package bytes, rather than links back to source files.

## API

- `previewDirectory(path)` and `importDirectory(path, expectedDigest)` are asynchronous. Import rereads the selected directory, computes its digest again and refuses changed content. Supported layouts are a root `SKILL.md`, a `skills/` tree, or sibling Skill directories; root LICENSE/NOTICE files remain included. ZIP/tar/remote installation is unsupported.
- `installBundled(manifestPath)` asynchronously reads the existing version-1 JSON manifest or gzip resource. The `every-compound-engineering` bundle keeps all 36 entries, 443 files, pinned source revision, source URL, license, per-file hashes and exact original bytes. Installation does not fetch that source URL. A changed manifest for an already retained package ID is a version conflict, not an implicit upgrade.
- `listPackages()` and `listSkills()` return installed catalog entries. Package views include `revision`; Skill views include persistent `id`, `packageId`, `entryPath`, `enabled` and `generation`. Lists do not grant guidance to any conversation.
- `setEnabled(skillId, enabled)` changes availability and increments generation only when the value changes. The package revision also advances. Disabled guidance remains inspectable in the management UI, matching the existing Rust resource API.
- `uninstallPackage(id, { expectedRevision })` requires the current package revision, hides the package and disables every entry while retaining copied bytes, provenance, IDs and generation tombstones. Reinstalling the exact content reuses IDs and advances generations again. It does not recover any conversation grant. New content with colliding names is rejected atomically; bundled names are namespaced only on first registration to preserve existing custom entries.
- `listResources(skillId)` exposes only that Skill's subtree plus root legal notices. Private configuration/key paths are excluded. Sibling instructions cannot be read through another Skill's ID.
- `readResource(skillId, path, offset = 0, limit = 4096)` returns the Rust-compatible `{ skillId, path, content, encoding, offset, nextOffset, hasMore, totalBytes }` shape. Limits are 1–8192 bytes. UTF-8 pages stop at complete code points; invalid offsets fail. Binary files are retained but return `content: null`. Complete bounded text is checked for credential patterns before any page is released. Scripts remain inert text.
- `fingerprint(skillId)` requires an installed, enabled entry. It hashes the current persistent generation, instruction metadata, package provenance and actual integrity-checked bytes of the entry's accessible resources. SQLite triggers advance that generation for actual instruction/provenance/resource changes, including changes later restored to their original bytes. Sibling-only resource changes do not revoke another entry; shared LICENSE/NOTICE changes affect every entry. Disabled/missing guidance and corrupt files fail closed. Fingerprints are opaque host-consent identities, not a portable grant or a publisher signature.
- `close()` closes the independent SQLite connection.

The host must bind selected Skill IDs and fingerprints to the actual conversation and recipient, and recheck before reading, transmitting or persisting model output. Turning off a Skill or uninstalling its package changes the fingerprint contract; bytes already sent to a provider cannot be recalled. This library intentionally contains no permissions table and no automatic tool discovery or execution authority.

## Compatibility and limits

The existing Rust `src-tauri/src/skill_packages` implementation defines the compatibility contract: metadata order and NUL-delimited SHA-256 package digest, per-file SHA-256, layouts, path/duplicate checks, resource scope, credential checks and UTF-8 byte pages. `SKILL.md` frontmatter reads only name and description; tool or permission claims remain ordinary untrusted text. Local packages use `local-content` and a content-addressed ID. Limits remain 512 files, 1 MiB per file, 16 MiB total, depth 16 and 128 Skills per package; instructions allow 30,000 Unicode code points.

Symlinks and special files are rejected. Import uses regular-file handles with no-follow flags, bounded reads and before/after metadata checks; Linux also verifies each opened file's actual `/proc/self/fd` location. This is bounded declarative import, not an operating-system sandbox. Content hashes detect changed bytes but do not authenticate an outside publisher or establish that a described workflow is supported by the host.

Run the local fixture suite with `node --test --test-concurrency=1 packages/skills-library/test/*.test.mjs`. Tests use temporary SQLite databases, synthetic folders and the checked-in inert bundle, without a provider or user data.
