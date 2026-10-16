import { isAbsolute } from 'node:path';
import { API_VERSION, PROTOCOL_VERSION, MAX_MESSAGE_BYTES, MAX_TOOL_RESULT_BYTES, AppSdkError, fail, jsonBytes, keys, plainObject, rpcError, validateContext } from './protocol.mjs';
import { validateManifest } from './manifest.mjs';
import { validateToolArguments } from './schema.mjs';

function resultValue(value) {
  keys(value, ['content', 'structuredContent', 'isError'], ['content'], 'tool result', 'app_result_invalid');
  if (!Array.isArray(value.content) || value.content.length > 32) fail('app_result_invalid', 'Tool content must be a bounded array');
  for (const part of value.content) {
    keys(part, ['type', 'text'], ['type', 'text'], 'tool content', 'app_result_invalid');
    if (part.type !== 'text' || typeof part.text !== 'string') fail('app_result_invalid', 'Only text tool content is supported');
  }
  if (value.isError !== undefined && typeof value.isError !== 'boolean') fail('app_result_invalid', 'isError must be boolean');
  jsonBytes(value, MAX_TOOL_RESULT_BYTES, 'app_result_invalid');
  return value;
}
export function createWorker(definition) {
  keys(definition, ['manifest', 'tools', 'onStart', 'onStop'], ['manifest', 'tools'], 'worker definition');
  const manifest = validateManifest(definition.manifest);
  if (!plainObject(definition.tools) || Object.keys(definition.tools).length !== manifest.tools.length
    || manifest.tools.some(tool => typeof definition.tools[tool.name] !== 'function')
    || Object.keys(definition.tools).some(name => !manifest.tools.some(tool => tool.name === name))) fail('app_worker_invalid', 'Tool handlers must exactly match the manifest');
  for (const hook of ['onStart', 'onStop']) if (definition[hook] !== undefined && typeof definition[hook] !== 'function') fail('app_worker_invalid', 'Lifecycle hooks must be functions');
  let state = 'new';
  let activeCalls = 0;
  const ids = new Set();
  async function dispatch(method, params) {
    if (method === 'describe') {
      keys(params, ['hostApiVersion'], ['hostApiVersion'], 'describe params');
      if (params.hostApiVersion !== API_VERSION) fail('app_host_api_incompatible', 'Unsupported host API version');
      return { protocolVersion: PROTOCOL_VERSION, appId: manifest.id, version: manifest.version,
        dataSchemaVersion: manifest.dataSchemaVersion, tools: manifest.tools, pages: manifest.pages };
    }
    if (method === 'lifecycle/start') {
      keys(params, ['dataDir', 'context'], ['dataDir', 'context'], 'start params');
      if (typeof params.dataDir !== 'string' || params.dataDir.length > 4096 || params.dataDir.includes('\0') || !isAbsolute(params.dataDir)) fail('app_data_dir_invalid', 'App data directory must be absolute');
      keys(params.context, ['hostApiVersion', 'appId'], ['hostApiVersion', 'appId'], 'start context');
      if (params.context.hostApiVersion !== API_VERSION || params.context.appId !== manifest.id) fail('app_host_api_incompatible', 'Start context does not match this app and host API');
      const context = Object.freeze({ ...params.context });
      if (state !== 'new') fail('app_lifecycle_invalid', 'App has already been started or stopped');
      state = 'starting';
      try {
        const started = await definition.onStart?.({ dataDir: params.dataDir, context });
        if (started !== undefined) {
          keys(started, ['dataSchemaVersion'], ['dataSchemaVersion'], 'start result', 'app_result_invalid');
          if (started.dataSchemaVersion !== manifest.dataSchemaVersion) fail('app_data_schema_mismatch', 'App data migration did not reach the declared version');
        }
        state = 'running';
        return { started: true, dataSchemaVersion: manifest.dataSchemaVersion };
      } catch (error) { state = 'failed'; throw error; }
    }
    if (method === 'lifecycle/stop') {
      keys(params, [], [], 'stop params');
      if (activeCalls > 0 || state === 'starting' || state === 'stopping') return { busy: true };
      if (state === 'stopped') return { busy: false };
      state = 'stopping';
      try {
        const stopped = await definition.onStop?.();
        if (stopped !== undefined) {
          keys(stopped, ['busy'], ['busy'], 'stop result', 'app_result_invalid');
          if (typeof stopped.busy !== 'boolean') fail('app_result_invalid', 'Stop busy must be boolean');
        }
        if (stopped?.busy === true) { state = 'running'; return { busy: true }; }
        state = 'stopped';
        return { busy: false };
      } catch (error) { state = 'failed'; throw error; }
    }
    if (method === 'tool/call') {
      keys(params, ['name', 'arguments', 'context'], ['name', 'arguments', 'context'], 'tool params');
      const context = validateContext(params.context);
      if (state !== 'running') fail('app_not_running', 'App is not running');
      const tool = manifest.tools.find(item => item.name === params.name);
      if (!tool) fail('app_tool_unknown', 'Tool is not declared');
      validateToolArguments(tool.inputSchema, params.arguments);
      if (activeCalls >= 16) fail('app_busy', 'App tool concurrency limit reached');
      activeCalls++;
      try { return resultValue(await definition.tools[tool.name](structuredClone(params.arguments), context)); }
      finally { activeCalls--; }
    }
    fail('app_method_unknown', 'Method is not supported', { rpcCode: -32601 });
  }
  return {
    async handle(request) {
      let id = null;
      try {
        jsonBytes(request, MAX_MESSAGE_BYTES);
        keys(request, ['jsonrpc', 'id', 'method', 'params'], ['jsonrpc', 'id', 'method', 'params'], 'request');
        if (request.jsonrpc !== '2.0' || (typeof request.id !== 'string' && !Number.isSafeInteger(request.id))
          || (typeof request.id === 'string' && (!request.id || request.id.length > 128)) || typeof request.method !== 'string') fail('app_request_invalid', 'Invalid JSON-RPC request', { rpcCode: -32600 });
        id = request.id;
        const key = `${typeof id}:${id}`;
        if (ids.has(key)) fail('app_request_duplicate', 'Request id is already in flight');
        ids.add(key);
        try {
          const result = await dispatch(request.method, request.params);
          jsonBytes(result, MAX_TOOL_RESULT_BYTES, 'app_result_invalid');
          return { jsonrpc: '2.0', id, result };
        }
        finally { ids.delete(key); }
      } catch (error) { return rpcError(id, error); }
    },
    get state() { return state; },
    get activeCalls() { return activeCalls; },
  };
}
export function startStdioWorker(definition, { input = process.stdin, output = process.stdout, diagnostics = process.stderr } = {}) {
  const worker = createWorker(definition);
  let pending = Buffer.alloc(0);
  let dropping = false;
  let ended = false;
  let outputFailed = false;
  const tasks = new Set();
  let resolveClosed;
  const closed = new Promise(resolve => { resolveClosed = resolve; });
  function finish() { if (ended && tasks.size === 0) resolveClosed(); }
  function send(value) {
    if (outputFailed) return;
    try { output.write(`${jsonBytes(value, MAX_MESSAGE_BYTES)}\n`); }
    catch { outputFailed = true; diagnostics.write('app_transport_output_failed\n'); }
  }
  function invalid(code, message, rpcCode) { send(rpcError(null, new AppSdkError(code, message, { rpcCode }))); }
  function line(bytes) {
    if (bytes.length === 0) { invalid('app_parse_error', 'Empty JSON-RPC line', -32700); return; }
    let request;
    try { request = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); }
    catch { invalid('app_parse_error', 'Request must be UTF-8 JSON', -32700); return; }
    if (tasks.size >= 32) { invalid('app_busy', 'App request concurrency limit reached', -32000); return; }
    const task = worker.handle(request).then(send).catch(() => invalid('app_internal_error', 'App operation failed', -32603));
    tasks.add(task);
    task.finally(() => { tasks.delete(task); finish(); });
  }
  function onData(chunk) {
    const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    let offset = 0;
    while (offset < bytes.length) {
      const newline = bytes.indexOf(10, offset);
      const end = newline < 0 ? bytes.length : newline;
      const piece = bytes.subarray(offset, end);
      if (!dropping && pending.length + piece.length > MAX_MESSAGE_BYTES) {
        pending = Buffer.alloc(0); dropping = true;
        invalid('app_message_too_large', 'Request exceeds its byte budget', -32600);
      }
      if (!dropping) pending = Buffer.concat([pending, piece]);
      if (newline < 0) break;
      if (!dropping) line(pending.at(-1) === 13 ? pending.subarray(0, -1) : pending);
      pending = Buffer.alloc(0); dropping = false; offset = newline + 1;
    }
  }
  function onEnd() {
    if (ended) return;
    ended = true;
    if (pending.length || dropping) invalid('app_parse_error', 'JSON-RPC lines must end with a newline', -32700);
    finish();
  }
  input.on('data', onData);
  input.once('end', onEnd);
  input.once('error', () => { diagnostics.write('app_transport_input_failed\n'); onEnd(); });
  output.on('error', () => { outputFailed = true; diagnostics.write('app_transport_output_failed\n'); });
  return { worker, closed, close() { input.off('data', onData); input.pause?.(); onEnd(); } };
}
