# Grok ACP runtime adapter

This dependency-free ESM package targets Node 24 or newer. It keeps one official Grok CLI process alive, transports ACP v1 JSON-RPC over newline-delimited UTF-8 stdin/stdout, and forwards incremental events. It does not implement a model loop, extract authentication tokens, execute app tools, read a user's Grok auth files, or call an authentication API itself.

The default command is `grok --no-auto-update agent stdio`. The host must deliberately connect it; constructing this class does not spawn anything. The official CLI must already be installed and authenticated in an explicitly approved private profile. `start()` negotiates protocol version `1`, requires advertised `cached_token`, then sends `authenticate` with `{methodId:"cached_token",_meta:{headless:true}}`. Credentials are neither requested nor accepted by the adapter. When the agent advertises only `grok.com` and/or `oidc` login methods, the adapter preserves error code `ACP_CACHED_AUTH_UNAVAILABLE` with `authenticationRequired:true` and `outcomeUnknown:false`; it sends no authentication, session or prompt request. The Host exposes `authentication_required` with a separate-login notice. Empty, unknown and API-key-only advertisements retain the prior explicit failure. The adapter never automatically chooses another authentication method or model.

The official CLI owns the behavior of `authenticate`: `headless:true` is not a promise that it will only inspect a cache. Reviewed official source can refresh an expired token and fall through from unavailable `cached_token` to another authentication flow. Do not use `grok.com` as a cache-check workaround, silently change a user's profile, or claim this adapter prevents the CLI's internal login fallback. A future strictly noninteractive profile policy needs explicit setup and validation; this change does not implement it. The isolated launch verified separately used `--no-auto-update agent --no-leader stdio` to avoid a shared leader.

```js
import { GrokAcpRuntime } from './index.mjs';

const runtime = new GrokAcpRuntime({
  cwd: '/absolute/approved/project',
  permissionPolicy: 'ask',
  onEvent(event) {
    // Route by sessionId + turnId; sequence is monotonic, generation identifies
    // the process connection. Persist bounded host-owned events as appropriate.
    if (event.type === 'text_delta') renderText(event.text);
    if (event.type === 'permission_requested') {
      // Display the offered options. An explicit user selection later calls:
      // runtime.resolvePermission(event.requestId, selectedOfficialOptionId).
      showPermission(event);
    }
  },
});

await runtime.start();
const { sessionId } = await runtime.newSession({
  cwd: '/absolute/approved/project',
  mcpServers: [{
    name: 'oviraptor-apps', command: process.execPath,
    args: ['/absolute/host/mcp-bridge.mjs'],
    env: [{ name: 'OVIRAPTOR_SCOPE', value: 'host-created-opaque-scope' }],
  }],
});
const result = await runtime.prompt({ sessionId, prompt: 'Summarize this project.' });
console.log(result.stopReason, result.metrics);
await runtime.close();
```

`mcpServers` is the standard ACP array and is forwarded as supplied on both `newSession()` and `loadSession()`. The host owns executable allowlisting and app-tool authorization, issues its own scope tokens, and binds the session to the correct project/conversation. The adapter never interprets or logs these environment values. Host-supplied MCP servers are executable configuration, not model-supplied data. No client file or terminal RPC capability is advertised; unsupported agent-to-client requests return JSON-RPC `-32601`.

## API

- `new GrokAcpRuntime(options)` accepts `cwd`, injectable `spawn(command,args,options)`, `command`, `args`, optional `env`, `onEvent`, optional `onPermission`, and limits. `spawn` uses `shell:false` and piped stdio. Overrides are trusted host/test configuration. The default request timeout is 30 seconds, prompt timeout 120 seconds, cancellation grace 5 seconds, permission deadline 30 seconds, and frame size limit 1 MiB. No test requires overriding the root package or installing dependencies.
- `start()` coalesces concurrent starts and returns `status`. Repeated starts while ready reuse the process. After a crash, an explicit `start()` creates a new process and authenticates it through the same official flow. A closed instance cannot restart.
- `newSession({cwd?,mcpServers?})` returns the agent's response including `sessionId`. Paths must be absolute; defaults are constructor `cwd` and an empty server array.
- `loadSession({sessionId,cwd?,mcpServers?})` requires `agentCapabilities.loadSession === true`. History notifications during the load carry `replay:true`; the adapter returns after the load response. A remembered session cannot be moved to a different working directory, including by concurrent loads. There is no invented fallback to a new session, no automatic prompt replay, and no `session/resume` fallback.
- `prompt({sessionId,prompt,turnId?,signal?,timeoutMs?})` accepts a string or an array of ACP text blocks. It returns `{sessionId,turnId,stopReason,metrics}`. Text arrives through events; the adapter does not assemble or retain a transcript. Prompts and loads are serialized per session; separate sessions can stream concurrently. An already-aborted queued prompt is never sent.
- `cancel(sessionId)` sends an ACP notification without an ID and returns whether cancellation was newly requested. It resolves outstanding and newly received permission requests as cancelled, keeps accepting final session updates, and waits for the original prompt response. A provider may finish before cancellation wins; its actual `stopReason` is retained. If the agent does not finish within the grace period, the process is killed, all pending requests reject, and existing sessions require explicit loading before another prompt.
- `resolvePermission(requestId,optionId)` selects one of the options supplied by the agent. The request ID is a runtime-local correlation key; the option ID remains the official ID. The default policy is `reject`: it chooses `reject_once`, then `reject_always`, or responds cancelled if no rejection option exists. `ask` waits for an explicit response up to the deadline, then denies. Invalid options, callback errors and missing decisions deny. An optional async `onPermission` may return an option ID or `{optionId}`; a callback returning nothing denies immediately. For event-driven UI, use `permissionPolicy:'ask'` and `resolvePermission`, without such a callback. A late or repeated decision returns `false` and cannot approve a cancelled request.
- `status` exposes state, negotiated capabilities/version, PID, timing and pending counts. A missing-login failure additionally exposes `authenticationRequired:true`; the unusable transport remains `failed`, while the Host presents the more specific `authentication_required` state. It contains no credentials or MCP configuration. `close()` cancels pending permission UI, rejects in-flight requests, sends SIGTERM, and uses bounded SIGKILL fallback.

Events include `runtime_ready`, `runtime_error`, `runtime_closed`, `session_ready`, `turn_started`, `text_delta`, `tool_call`, `tool_update`, `session_update`, `permission_requested`, `permission_resolved`, `cancel_requested`, `turn_complete`, and `turn_error`. Session events contain `sessionId`; turn events contain `turnId`. Updates retain the original `update` data, including non-text notifications. Unknown notifications expose only their method name. Events are synchronous to deliver deltas promptly; host callback failures are counted and isolated. The host must keep callbacks fast and bound any persistence queue.

Each successful turn reports `coldStartMs`, `firstTextMs`, `firstToolMs`, and `totalMs`. Cold start covers spawn, initialization and cached authentication. First text/tool and total are measured from the prompt request using a monotonic clock; absent text/tools stay `null`. Turn failures also emit measured performance. These are instrumentation fields, not claims about actual Grok latency.

Malformed frames, invalid envelopes, unexpected/duplicate responses, oversized frames, and transport errors fail closed. Timeouts cancel first where applicable and never retry a prompt. Process-generation checks prevent old stream frames, write errors, late responses and synchronous host callbacks from reviving a dead connection or killing a newer one. Raw stderr is discarded (only its byte count is kept); raw remote error messages/data are replaced with safe error codes because they may contain secrets. Agent text and tool-update content remain untrusted application data for the host to render and handle safely.

## Validation and boundaries

Run from the repository root:

```sh
node --test packages/runtime-acp/tests/*.test.mjs
```

Tests spawn `tests/fixtures/agent.mjs` as a real local Node process. They exercise reuse, per-session serialization and concurrent streams, split UTF-8 NDJSON, incremental text/tools before completion, approval defaults and explicit selection, permission cancellation and late decisions, AbortSignal, prompt/cancel timeouts, process crashes, explicit capability-gated restoration, malformed protocol, and scoped MCP configuration. In-memory process mocks target restart/reentrancy races that are hard to schedule deterministically in a real CLI. Fixtures use no network, credentials, auth cache, user files, or actual Grok process.

These fixtures do not verify cached authentication success, real model/tool behavior, native packaging, MCP child execution by the official CLI, or live cancellation latency. A separate isolated official npm Grok 1.0.60 check on 2026-10-16 sent only `initialize`: it negotiated protocol 1 and advertised `loadSession` plus HTTP/SSE MCP support, but offered only `grok.com` with no default auth method. It then exited normally after EOF. No real `authenticate`, session creation, login or model prompt was sent. Advertised capabilities and offline model metadata do not prove those operations work or that a model account is ready. The reviewed public source checkout is not proven to reproduce that exact npm binary.

## Official protocol references

- [x.ai headless scripting and ACP](https://docs.x.ai/build/cli/headless-scripting): official stdio command, cached authentication example, streamed `session/update`, and `--no-auto-update`.
- [ACP v1 initialization](https://agentclientprotocol.com/protocol/v1/initialization) and [v1 schema](https://agentclientprotocol.com/protocol/v1/schema): protocol version negotiation and exact request/response fields. ACP v2 is a draft and is not used here.
- [Session setup](https://agentclientprotocol.com/protocol/v1/session-setup), [prompt lifecycle](https://agentclientprotocol.com/protocol/v1/prompt-turn), [cancellation](https://agentclientprotocol.com/protocol/v1/cancellation), [tools and permission outcomes](https://agentclientprotocol.com/protocol/v1/tool-calls), and [NDJSON stdio transport](https://agentclientprotocol.com/protocol/v1/transports).
