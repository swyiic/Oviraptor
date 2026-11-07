import test from 'node:test';
import assert from 'node:assert/strict';
import { workspacePermission } from '../src/workspace-tools.mjs';

// Safety invariant: outward tools (network / browser / host shell) are a DISTINCT
// permission class from sandboxed analysis, so a one-click "allow out-of-workspace
// access" grant (read + no-net run + write) never also unlocks these.
test('outward tools are workspace.network', () => {
  for (const name of ['workspace_fetch', 'workspace_http', 'workspace_browser', 'workspace_ws', 'workspace_shell', 'workspace_probe_sql', 'workspace_probe_command'])
    assert.equal(workspacePermission(name), 'workspace.network', name);
});

test('sandboxed-analysis tools are read/write/run — NEVER network', () => {
  assert.equal(workspacePermission('workspace_run'), 'workspace.run');
  for (const name of ['workspace_write', 'workspace_edit']) assert.equal(workspacePermission(name), 'workspace.write', name);
  for (const name of ['workspace_list', 'workspace_read', 'workspace_grep', 'workspace_glob', 'workspace_read_path', 'workspace_source_list', 'workspace_source_search', 'workspace_chain_read', 'workspace_chain_add'])
    assert.equal(workspacePermission(name), 'workspace.read', name);
  for (const name of ['workspace_run', 'workspace_write', 'workspace_edit', 'workspace_read', 'workspace_read_path', 'workspace_grep', 'workspace_glob', 'workspace_list'])
    assert.notEqual(workspacePermission(name), 'workspace.network', name);
});

test('unknown tool throws', () => { assert.throws(() => workspacePermission('workspace_definitely_not_real')); });
