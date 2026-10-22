import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { AgentHost } from '../src/service.mjs';
import { createDirectoryPackage } from '../../app-sdk/src/index.mjs';

const repository = fileURLToPath(new URL('../../../', import.meta.url));
for (const combination of [[], ['assets'], ['scanning'], ['assets', 'scanning']]) {
  test(`independent core with installed applications: ${combination.join('+') || 'none'}`, async t => {
    const directory = await mkdtemp(join(tmpdir(), 'oviraptor-app-combination-'));
    const host = new AgentHost({ dataDir: join(directory, 'host') });
    t.after(async () => { await host.close(); await rm(directory, { recursive: true, force: true }); });
    const project = host.store.createProject('Synthetic combination'), other = host.store.createProject('Synthetic other');
    for (const name of combination) {
      const sourceDirectory = join(repository, 'apps', name);
      const spec = JSON.parse(await readFile(join(sourceDirectory, 'app.package.json'), 'utf8'));
      const outputDirectory = join(directory, `${name}-package`);
      await createDirectoryPackage({ ...spec, sourceDirectory, outputDirectory });
      const installed = await host.apps.install(outputDirectory);
      await host.apps.setEnabled(installed.id, true);
      assert.equal(host.apps.capabilities(project.id).some(tool => tool.appId === installed.id), false);
      host.apps.setGrants(installed.id, project.id, spec.manifest.permissions.map(permission => permission.id));
      if (name === 'assets') {
        await host.apps.call(installed.id, 'inventory_import', project.id, { source: 'synthetic-test', records: [{ externalId: 'a', host: 'asset.example.test' }] });
        assert.equal((await host.apps.call(installed.id, 'inventory_stats', project.id, {})).structuredContent.total, 1);
        host.apps.setGrants(installed.id, other.id, ['inventory.read']);
        assert.equal((await host.apps.call(installed.id, 'inventory_stats', other.id, {})).structuredContent.total, 0);
      } else {
        const draft = (await host.apps.call(installed.id, 'draft_create', project.id, { title: 'Synthetic draft only', targets: ['https://no-network.example.test/'] })).structuredContent;
        assert.equal(draft.status, 'draft'); assert.equal(draft.executionStarted, false); assert.equal(draft.executionAvailable, false);
        const journal = (await host.apps.call(installed.id, 'journal_read', project.id, { taskId: draft.id })).structuredContent;
        assert.equal(journal.task.id, draft.id);
        host.apps.setGrants(installed.id, other.id, ['investigation.read']);
        await assert.rejects(host.apps.call(installed.id, 'journal_read', other.id, { taskId: draft.id }));
      }
    }
    assert.equal(host.apps.list().length, combination.length);
    const conversation = host.store.createConversation(project.id);
    assert.equal((await host.send(conversation.id, { projectId: project.id, text: 'Core chat remains available' })).savedOnly, true);
    const tables = host.store.db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").all().map(row => row.name);
    assert.deepEqual(tables, ['application_grants', 'application_retained', 'application_versions', 'applications', 'conversation_memory', 'conversation_tasks', 'conversation_workspace_grants', 'conversations', 'host_budget_entries', 'host_budget_limits', 'host_settings', 'human_reviews', 'messages', 'native_application_owners', 'project_access_policies', 'project_allocations', 'projects', 'provider_usage', 'report_versions', 'runtime_metrics', 'summary_versions', 'task_approvals', 'tool_records']);
    assert.deepEqual(host.store.db.prepare('PRAGMA table_info(conversation_tasks)').all().map(row => row.name), ['id', 'conversation_id', 'payload', 'created_at', 'updated_at']);
    const task = host.store.task(conversation.id);
    assert.equal(task.conversationId, conversation.id); assert.equal(task.projectId, project.id); assert.equal(task.goal, 'Core chat remains available');
    assert.deepEqual([task.steps, task.evidence, task.artifacts, task.runs], [[], [], [], []]);
    assert.equal(task.status, 'awaiting_input');
    assert.equal(host.store.db.prepare('SELECT COUNT(*) AS n FROM conversation_tasks').get().n, 1, 'application inventory/scan operations never create Host chat task records');
    assert.doesNotMatch(JSON.stringify(task), /asset\.example\.test|no-network\.example\.test/, 'Host task metadata does not mirror application inventory or scan targets');
  });
}
