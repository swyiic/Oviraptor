#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const input = JSON.parse(fs.readFileSync(0, "utf8"));
const targetUrl = String(input.url || "").trim();
const authSession = input.authSession && typeof input.authSession === "object" ? input.authSession : {};
const pageTimeoutMs = Math.max(5000, Number(input.timeoutMs || 15000));
const explorationTimeoutMs = Math.max(12000, Number(input.explorationTimeoutMs || 45000));
const maxActions = Math.max(0, Math.min(80, Number(input.maxActions ?? 24)));
const maxStates = Math.max(1, Math.min(40, Number(input.maxStates ?? 12)));
const maxDepth = Math.max(0, Math.min(5, Number(input.maxDepth ?? 2)));
const settleMs = Math.max(250, Math.min(3000, Number(input.settleMs ?? 750)));
const maxRequests = Math.max(50, Math.min(4000, Number(input.maxRequests ?? 800)));
const empty = {
  available: false,
  browser: "",
  frameworks: [],
  routes: [],
  scripts: [],
  requests: [],
  links: [],
  linkRecords: [],
  forms: [],
  navigations: [],
  states: [],
  actions: [],
  features: [],
  blockedRequests: [],
  authSessionValidation: { applied: false, valid: false, clearSessionInvalid: false, wafDetected: false, reason: "no_session" },
  coverage: { stateCount: 0, actionCount: 0, requestCount: 0 },
  stopReason: "unavailable",
  durationMs: 0,
  errors: [],
};

function browserCandidates() {
  const candidates = [process.env.OVIRAPTOR_BROWSER_EXECUTABLE].filter(Boolean);
  if (process.platform === "darwin") {
    candidates.push(
      "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
      "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
      "/Applications/Chromium.app/Contents/MacOS/Chromium",
    );
  } else if (process.platform === "win32") {
    for (const root of [process.env.PROGRAMFILES, process.env["PROGRAMFILES(X86)"], process.env.LOCALAPPDATA]) {
      if (!root) continue;
      candidates.push(
        path.join(root, "Google/Chrome/Application/chrome.exe"),
        path.join(root, "Microsoft/Edge/Application/msedge.exe"),
        path.join(root, "Chromium/Application/chrome.exe"),
      );
    }
  } else {
    candidates.push(
      "/usr/bin/google-chrome",
      "/usr/bin/google-chrome-stable",
      "/usr/bin/microsoft-edge",
      "/usr/bin/microsoft-edge-stable",
      "/usr/bin/chromium",
      "/usr/bin/chromium-browser",
    );
  }
  return [...new Set(candidates)].find((candidate) => candidate && fs.existsSync(candidate)) || "";
}

function dedupe(items, key) {
  const seen = new Set();
  return items.filter((item) => {
    const marker = key(item);
    if (!marker || seen.has(marker)) return false;
    seen.add(marker);
    return true;
  });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function bodyKeys(postData) {
  const value = String(postData || "").slice(0, 12000);
  if (!value) return [];
  try {
    const parsed = JSON.parse(value);
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) return Object.keys(parsed).slice(0, 80);
  } catch {}
  try {
    return [...new URLSearchParams(value).keys()].slice(0, 80);
  } catch {}
  return dedupe([...value.matchAll(/(?:^|[,{&?\s])([A-Za-z_$][\w$.-]{1,80})\s*[:=]/g)].map((item) => item[1]), String).slice(0, 80);
}

function queryKeys(value) {
  try {
    return [...new URL(value).searchParams.keys()].slice(0, 80);
  } catch {
    return [];
  }
}

function boundedHeaders(headers) {
  return Object.fromEntries(
    Object.entries(headers || {})
      .slice(0, 100)
      .map(([key, value]) => [String(key), String(value).slice(0, 4000)]),
  );
}

function mergedHeaders(...values) {
  const result = {};
  for (const headers of values) {
    for (const [key, value] of Object.entries(headers || {})) result[String(key)] = String(value).slice(0, 4000);
  }
  return boundedHeaders(result);
}

function normalizedHost(value) {
  try { return new URL(value).hostname.toLowerCase(); } catch { return ""; }
}

function hostInScope(host, scopes) {
  return (scopes || []).some((value) => {
    const scope = String(value || "").replace(/^\./, "").toLowerCase();
    return scope && (host === scope || host.endsWith(`.${scope}`));
  });
}

function loginLike(value) {
  const lower = String(value || "").toLowerCase();
  return ["/login", "/signin", "/sign-in", "/auth/login", "/passport/login", "cas/login", "sso/login", "oauth/authorize"]
    .some((marker) => lower.includes(marker));
}

function wafMarker(value) {
  const lower = String(value || "").toLowerCase();
  return [
    "cf-chl-", "cloudflare ray id", "attention required! | cloudflare", "aws waf", "akamai reference",
    "incapsula incident id", "sucuri website firewall", "web application firewall", "js challenge",
    "verify you are human", "人机验证", "访问验证",
  ].some((marker) => lower.includes(marker));
}

function confirmedWaf(requestItems, bodyPreview = "") {
  if (wafMarker(bodyPreview)) return true;
  const recent = requestItems.slice(-30);
  if (recent.some((item) => wafMarker(JSON.stringify(item.responseHeaders || {})) || wafMarker(item.responsePreview || ""))) return true;
  const limited = recent.filter((item) => Number(item.status || 0) === 429).length;
  return limited >= 3;
}

function headerDifference(full, visible) {
  const known = new Set(Object.keys(visible || {}).map((key) => key.toLowerCase()));
  return Object.keys(full || {}).filter((key) => !known.has(key.toLowerCase())).slice(0, 100);
}

function compactInitiator(initiator) {
  const frames = initiator?.stack?.callFrames || [];
  return {
    type: String(initiator?.type || ""),
    url: String(initiator?.url || frames[0]?.url || "").slice(0, 1200),
    lineNumber: Number(initiator?.lineNumber ?? frames[0]?.lineNumber ?? -1),
    columnNumber: Number(initiator?.columnNumber ?? frames[0]?.columnNumber ?? -1),
    functionName: String(frames[0]?.functionName || "").slice(0, 240),
    stack: frames.slice(0, 12).map((frame) => ({
      functionName: String(frame.functionName || "").slice(0, 240),
      url: String(frame.url || "").slice(0, 1200),
      lineNumber: Number(frame.lineNumber ?? -1),
      columnNumber: Number(frame.columnNumber ?? -1),
    })),
  };
}

function sameOriginUrl(value, origin) {
  try {
    const parsed = new URL(value, origin);
    if (parsed.origin !== origin || !/^https?:$/.test(parsed.protocol)) return "";
    parsed.username = "";
    parsed.password = "";
    return parsed.href;
  } catch {
    return "";
  }
}

const navigationNoiseKeys = /^(?:lang|locale|language|lng|hl|utm_.+|gclid|fbclid|ref|source|tracking)$/i;
const valuableNavigation = /(?:admin|dashboard|account|profile|user|role|permission|member|auth|login|password|reset|register|upload|download|export|api|graphql|file|document|order|payment|config|setting|system|audit|report|search|detail|管理|账户|用户|角色|权限|登录|注册|上传|下载|导出|接口|文件|订单|支付|配置|审计|报表|详情)/i;
const lowValueNavigation = /(?:^|\/)(?:i18n|locale|locales|language|languages)(?:\/|$)/i;
const documentationNavigation = /(?:^|\/)(?:help|docs?|documentation)(?:\/|$)/i;

function explorationKey(value, origin) {
  const normalized = sameOriginUrl(value, origin);
  if (!normalized) return "";
  const parsed = new URL(normalized);
  const pathShape = parsed.pathname
    .replace(/\/[0-9]{2,}(?=\/|$)/g, "/<id>")
    .replace(/\/[0-9a-f]{8}-[0-9a-f-]{27,}(?=\/|$)/gi, "/<uuid>")
    .replace(/\/{2,}/g, "/");
  const queryShape = [...new Set([...parsed.searchParams.keys()]
    .filter((key) => !navigationNoiseKeys.test(key)))]
    .sort()
    .join("&");
  const hashShape = parsed.hash.startsWith("#/")
    ? `#${parsed.hash.slice(1).split("?")[0].replace(/\/[0-9]{2,}(?=\/|$)/g, "/<id>")}`
    : "";
  return `${pathShape}?${queryShape}${hashShape}`;
}

function explorationPriority(value, source, depth) {
  const parsed = new URL(value);
  const searchable = `${parsed.pathname} ${[...parsed.searchParams.keys()].join(" ")} ${source}`;
  let score = Math.max(0, 80 - depth * 10);
  if (valuableNavigation.test(searchable)) score += 100;
  if (String(source).startsWith("router:") || String(source).startsWith("action:")) score += 30;
  if (lowValueNavigation.test(parsed.pathname)) score -= 140;
  if (parsed.search && [...parsed.searchParams.keys()].every((key) => navigationNoiseKeys.test(key))) score -= 100;
  return score;
}

async function main() {
  const startedAt = Date.now();
  if (!/^https?:\/\//i.test(targetUrl)) {
    process.stdout.write(JSON.stringify({ ...empty, errors: ["invalid_target_url"] }));
    return;
  }
  const executable = browserCandidates();
  if (!executable) {
    process.stdout.write(JSON.stringify({
      ...empty,
      errors: ["未找到 Google Chrome、Microsoft Edge 或 Chromium；运行时补全已跳过。"],
    }));
    return;
  }

  const profileDir = fs.mkdtempSync(path.join(os.tmpdir(), "oviraptor-runtime-"));
  const child = spawn(executable, [
    "--headless=new",
    "--remote-debugging-pipe",
    `--user-data-dir=${profileDir}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    "--disable-features=Translate,OptimizationHints,MediaRouter",
    "--disable-popup-blocking",
    "--ignore-certificate-errors",
    "about:blank",
  ], {
    stdio: ["ignore", "ignore", "pipe", "pipe", "pipe"],
  });

  const commandPipe = child.stdio[3];
  const eventPipe = child.stdio[4];
  let nextId = 0;
  let buffer = Buffer.alloc(0);
  const pending = new Map();
  const listeners = new Set();
  const stderr = [];

  child.stderr.on("data", (chunk) => stderr.push(String(chunk)));
  eventPipe.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    let marker;
    while ((marker = buffer.indexOf(0)) >= 0) {
      const raw = buffer.subarray(0, marker).toString("utf8");
      buffer = buffer.subarray(marker + 1);
      if (!raw.trim()) continue;
      let message;
      try {
        message = JSON.parse(raw);
      } catch {
        continue;
      }
      if (message.id && pending.has(message.id)) {
        const waiter = pending.get(message.id);
        pending.delete(message.id);
        clearTimeout(waiter.timer);
        if (message.error) waiter.reject(new Error(message.error.message || JSON.stringify(message.error)));
        else waiter.resolve(message.result || {});
      } else {
        for (const listener of listeners) listener(message);
      }
    }
  });

  function send(method, params = {}, sessionId = undefined) {
    const id = ++nextId;
    const message = { id, method, params };
    if (sessionId) message.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, Math.max(10000, pageTimeoutMs));
      pending.set(id, { resolve, reject, timer });
      commandPipe.write(`${JSON.stringify(message)}\0`);
    });
  }

  const requests = [];
  const requestById = new Map();
  const pendingRequestExtra = new Map();
  const pendingResponseExtra = new Map();
  const blockedRequests = [];
  const scripts = [];
  const responseReads = [];
  let responseReadCount = 0;
  let sessionId = "";
  let activeContext = { actionId: "initial-load", stateId: "state-0", feature: "entry-page" };

  function applyRequestExtra(record, params) {
    if (!record || !params) return;
    const original = record.headers || {};
    const effective = mergedHeaders(original, params.headers || {});
    record.effectiveRequestHeaders = effective;
    record.effectiveRequestHeaderNames = Object.keys(effective).slice(0, 120);
    record.extraInfoRequestHeaderNames = Object.keys(params.headers || {}).slice(0, 120);
    record.extraRequestHeaderNames = headerDifference(effective, original);
    record.associatedCookies = (params.associatedCookies || []).map((item) => ({
      name: String(item.cookie?.name || ""),
      domain: String(item.cookie?.domain || ""),
      path: String(item.cookie?.path || ""),
      sameSite: String(item.cookie?.sameSite || ""),
      blockedReasons: (item.blockedReasons || []).map(String),
    })).filter((item) => item.name).slice(0, 80);
    record.connectTiming = params.connectTiming || {};
    record.clientSecurityState = params.clientSecurityState || {};
  }

  function applyResponseExtra(record, params) {
    if (!record || !params) return;
    const original = record.responseHeaders || {};
    const effective = mergedHeaders(original, params.headers || {});
    record.effectiveResponseHeaders = effective;
    record.effectiveResponseHeaderNames = Object.keys(effective).slice(0, 120);
    record.extraResponseHeaderNames = headerDifference(effective, original);
    record.responseHeadersText = String(params.headersText || "").slice(0, 16_000);
    record.blockedResponseCookies = (params.blockedCookies || []).map((item) => ({
      name: String(item.cookieLine || "").split("=", 1)[0].trim(),
      blockedReasons: (item.blockedReasons || []).map(String),
    })).filter((item) => item.name).slice(0, 80);
  }

  function observedRequest(request, resourceType, requestId, source = "browser-runtime") {
    const record = {
      requestId: String(requestId || ""),
      url: String(request.url || ""),
      method: String(request.method || "GET").toUpperCase(),
      resourceType: String(resourceType || ""),
      source,
      queryKeys: queryKeys(request.url),
      bodyKeys: bodyKeys(request.postData),
      postData: String(request.postData || "").slice(0, 12000),
      headerNames: Object.keys(request.headers || {}).slice(0, 80),
      headers: boundedHeaders(request.headers),
      actionId: activeContext.actionId,
      stateId: activeContext.stateId,
      feature: activeContext.feature,
    };
    requests.push(record);
    if (requestId) {
      const key = String(requestId);
      requestById.set(key, record);
      if (pendingRequestExtra.has(key)) {
        applyRequestExtra(record, pendingRequestExtra.get(key));
        pendingRequestExtra.delete(key);
      }
      if (pendingResponseExtra.has(key)) {
        applyResponseExtra(record, pendingResponseExtra.get(key));
        pendingResponseExtra.delete(key);
      }
    }
    return record;
  }

  listeners.add((message) => {
    if (message.sessionId && sessionId && message.sessionId !== sessionId) return;
    if (message.method === "Network.requestWillBeSent" && requests.length < maxRequests) {
      const request = message.params?.request || {};
      const type = String(message.params?.type || "");
      if (request.url) {
        const record = observedRequest(request, type, message.params?.requestId);
        record.documentUrl = String(message.params?.documentURL || "").slice(0, 1200);
        record.frameId = String(message.params?.frameId || "");
        record.loaderId = String(message.params?.loaderId || "");
        record.hasUserGesture = Boolean(message.params?.hasUserGesture);
        record.initiator = compactInitiator(message.params?.initiator || {});
        const staticNonScript = /\.(?:css|map|png|jpe?g|gif|svg|webp|avif|woff2?|ttf|eot|ico)(?:[?#]|$)/i.test(request.url);
        if (!staticNonScript && (type === "Script" || /\.(?:m?js|jsx|tsx?)(?:[?#]|$)/i.test(request.url))) scripts.push(request.url);
      }
    }
    if (message.method === "Network.requestWillBeSentExtraInfo") {
      const key = String(message.params?.requestId || "");
      const record = requestById.get(key);
      if (record) applyRequestExtra(record, message.params || {});
      else if (key) pendingRequestExtra.set(key, message.params || {});
    }
    if (message.method === "Network.responseReceived") {
      const record = requestById.get(String(message.params?.requestId || ""));
      const response = message.params?.response || {};
      if (record) {
        record.status = Number(response.status || 0);
        record.statusText = String(response.statusText || "");
        record.contentType = String(response.mimeType || "");
        record.responseHeaderNames = Object.keys(response.headers || {}).slice(0, 80);
        record.responseHeaders = boundedHeaders(response.headers);
        record.remoteIpAddress = String(response.remoteIPAddress || "");
        record.fromServiceWorker = Boolean(response.fromServiceWorker);
        record.fromDiskCache = Boolean(response.fromDiskCache);
        record.fromPrefetchCache = Boolean(response.fromPrefetchCache);
        record.protocol = String(response.protocol || "");
      }
    }
    if (message.method === "Network.responseReceivedExtraInfo") {
      const key = String(message.params?.requestId || "");
      const record = requestById.get(key);
      if (record) applyResponseExtra(record, message.params || {});
      else if (key) pendingResponseExtra.set(key, message.params || {});
    }
    if (message.method === "Network.webSocketCreated" && requests.length < maxRequests) {
      const requestId = String(message.params?.requestId || "");
      if (message.params?.url && !requestById.has(requestId)) {
        observedRequest({ url: message.params.url, method: "GET", headers: {} }, "WebSocket", requestId, "browser-runtime-websocket");
      }
    }
    if (message.method === "Network.webSocketWillSendHandshakeRequest") {
      const key = String(message.params?.requestId || "");
      const record = requestById.get(key);
      if (record) applyRequestExtra(record, { headers: message.params?.request?.headers || {} });
    }
    if (message.method === "Network.webSocketHandshakeResponseReceived") {
      const key = String(message.params?.requestId || "");
      const record = requestById.get(key);
      const response = message.params?.response || {};
      if (record) {
        record.status = Number(response.status || 0);
        record.statusText = String(response.statusText || "");
        record.responseHeaders = boundedHeaders(response.headers || {});
        applyResponseExtra(record, { headers: response.headers || {}, headersText: response.headersText || "" });
      }
    }
    if (message.method === "Network.loadingFinished") {
      const record = requestById.get(String(message.params?.requestId || ""));
      if (record) {
        record.encodedDataLength = Number(message.params?.encodedDataLength || 0);
        const type = String(record.resourceType || "").toLowerCase();
        const contentType = String(record.contentType || "").toLowerCase();
        const readable = ["xhr", "fetch"].includes(type)
          && !/(?:image|audio|video|font|octet-stream|zip|pdf)/i.test(contentType)
          && record.encodedDataLength <= 256_000
          && responseReadCount < 80;
        if (readable) {
          responseReadCount += 1;
          const read = send("Network.getResponseBody", { requestId: message.params.requestId }, sessionId)
            .then((result) => {
              if (result.base64Encoded) return;
              const preview = String(result.body || "").slice(0, 12_000);
              record.responsePreview = preview;
              record.responseKeys = bodyKeys(preview);
            })
            .catch(() => {});
          responseReads.push(read);
        }
      }
    }
    if (message.method === "Fetch.requestPaused") {
      const request = message.params?.request || {};
      const method = String(request.method || "GET").toUpperCase();
      if (!requestById.has(String(message.params?.networkId || "")) && requests.length < maxRequests) {
        observedRequest(request, message.params?.resourceType || "Fetch", message.params?.networkId, "browser-runtime-intercept");
      }
      const captureOnly = Boolean(activeContext.captureOnly);
      if (captureOnly || !["GET", "HEAD", "OPTIONS"].includes(method)) {
        blockedRequests.push({
          url: String(request.url || ""), method,
          resourceType: String(message.params?.resourceType || ""),
          queryKeys: queryKeys(request.url), bodyKeys: bodyKeys(request.postData),
          postData: String(request.postData || "").slice(0, 12000),
          headers: boundedHeaders(request.headers),
          actionId: activeContext.actionId, stateId: activeContext.stateId,
          reason: captureOnly ? "capture_only_action_observed_without_forwarding" : "mutation_observed_without_forwarding",
        });
        void send("Fetch.failRequest", { requestId: message.params.requestId, errorReason: "Aborted" }, sessionId).catch(() => {});
      } else {
        void send("Fetch.continueRequest", { requestId: message.params.requestId }, sessionId).catch(() => {});
      }
    }
  });

  const initScript = String.raw`
(() => {
  const state = window.__OVIRAPTOR_RUNTIME__ = window.__OVIRAPTOR_RUNTIME__ || {
    vueApps: [], vueVersions: [], reactRenderers: [], requests: [], navigations: []
  };
  const originalPush = history.pushState.bind(history);
  const originalReplace = history.replaceState.bind(history);
  history.pushState = function(s, t, u) { if (u != null) state.navigations.push(String(u)); return originalPush(s, t, u); };
  history.replaceState = function(s, t, u) { if (u != null) state.navigations.push(String(u)); return originalReplace(s, t, u); };
  const originalFetch = window.fetch;
  if (originalFetch) window.fetch = function(requestInput, init) {
    try { state.requests.push({url: String(requestInput && requestInput.url || requestInput), method: String(init && init.method || "GET"), resourceType: "Fetch"}); } catch {}
    return originalFetch.apply(this, arguments);
  };
  const originalOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method, url) {
    try { state.requests.push({url: String(url), method: String(method || "GET"), resourceType: "XHR"}); } catch {}
    return originalOpen.apply(this, arguments);
  };
  if (!window.__REACT_DEVTOOLS_GLOBAL_HOOK__) {
    const renderers = new Map();
    window.__REACT_DEVTOOLS_GLOBAL_HOOK__ = {
      supportsFiber: true, renderers,
      inject(renderer) {
        const id = renderers.size + 1;
        renderers.set(id, renderer);
        try { state.reactRenderers.push({version: String(renderer.version || ""), rendererPackageName: String(renderer.rendererPackageName || "react-dom")}); } catch {}
        return id;
      },
      onCommitFiberRoot() {}, onCommitFiberUnmount() {}, onPostCommitFiberRoot() {}
    };
  }
  if (!window.__VUE_DEVTOOLS_GLOBAL_HOOK__) {
    const callbacks = new Map();
    window.__VUE_DEVTOOLS_GLOBAL_HOOK__ = {
      enabled: true, apps: [],
      on(event, fn) { const list = callbacks.get(event) || []; list.push(fn); callbacks.set(event, list); },
      once(event, fn) { const wrap = (...args) => { this.off(event, wrap); fn(...args); }; this.on(event, wrap); },
      off(event, fn) { callbacks.set(event, (callbacks.get(event) || []).filter((item) => item !== fn)); },
      emit(event, ...args) {
        if (event === "app:init" && args[0]) {
          this.apps.push(args[0]); state.vueApps.push(args[0]);
          if (args[1]) state.vueVersions.push(String(args[1]));
        }
        for (const fn of callbacks.get(event) || []) { try { fn(...args); } catch {} }
      }
    };
  }
})();`;

  const evaluation = String.raw`
(() => {
  const state = window.__OVIRAPTOR_RUNTIME__ || {};
  const frameworks = [];
  const routes = [];
  const apps = [...(state.vueApps || [])];
  const roots = [];
  const seenRoots = new Set();
  const collectRoot = (root) => {
    if (!root || seenRoots.has(root)) return;
    seenRoots.add(root); roots.push(root);
    for (const element of Array.from(root.querySelectorAll?.("*") || [])) {
      if (element.shadowRoot) collectRoot(element.shadowRoot);
      if (element.tagName === "IFRAME") {
        try { if (element.contentDocument) collectRoot(element.contentDocument); } catch {}
      }
    }
  };
  collectRoot(document);
  const selectAll = (selector) => roots.flatMap((root) => Array.from(root.querySelectorAll?.(selector) || []));
  const nodes = selectAll("*");
  for (const node of nodes) {
    if (node.__vue_app__) apps.push(node.__vue_app__);
    if (node.__vue__ && node.__vue__.$root) apps.push(node.__vue__.$root);
  }
  const seenApps = new Set();
  for (const app of apps) {
    if (!app || seenApps.has(app)) continue;
    seenApps.add(app);
    const version = String(app.version || app.$options?._base?.version || app.constructor?.version || "");
    frameworks.push({name: "Vue", version, evidence: "rendered Vue application instance"});
    const router = app.config?.globalProperties?.$router || app._instance?.appContext?.config?.globalProperties?.$router ||
      app.$router || app.$options?.router || app._router || app.$root?.$router || app.$root?.$options?.router;
    let records = [];
    try {
      if (typeof router?.getRoutes === "function") records = router.getRoutes();
      else if (Array.isArray(router?.options?.routes)) records = router.options.routes;
      else if (typeof router?.matcher?.getRoutes === "function") records = router.matcher.getRoutes();
    } catch {}
    const visit = (items, parent = "") => {
      for (const item of Array.isArray(items) ? items : []) {
        const raw = String(item?.path || "");
        const full = raw.startsWith("/") ? raw : ((parent === "/" ? "" : parent.replace(/\/+$/, "")) + "/" + raw).replace(/\/{2,}/g, "/");
        if (full) routes.push({path: full || "/", rawPath: raw, parentPath: parent, name: String(item?.name || ""), source: "browser-runtime", type: "frontend", confidence: "high", extractionEngine: "browser-runtime"});
        if (Array.isArray(item?.children)) visit(item.children, full || parent);
      }
    };
    visit(records);
  }
  for (const version of state.vueVersions || []) frameworks.push({name: "Vue", version: String(version), evidence: "Vue devtools app:init"});
  const renderers = window.__REACT_DEVTOOLS_GLOBAL_HOOK__?.renderers;
  if (renderers?.forEach) renderers.forEach((renderer) => frameworks.push({name: "React", version: String(renderer?.version || ""), evidence: String(renderer?.rendererPackageName || "React renderer")}));
  for (const renderer of state.reactRenderers || []) frameworks.push({name: "React", version: String(renderer.version || ""), evidence: String(renderer.rendererPackageName || "React renderer")});
  const angularRoot = selectAll("[ng-version]")[0];
  if (angularRoot) frameworks.push({name: "Angular", version: String(angularRoot.getAttribute("ng-version") || ""), evidence: "DOM ng-version"});
  if (window.__NEXT_DATA__) frameworks.push({name: "React", version: "", evidence: "Next.js __NEXT_DATA__"});
  if (window.__NUXT__ || selectAll("#__nuxt").length) frameworks.push({name: "Vue", version: "", evidence: "Nuxt runtime"});
  if (nodes.some((node) => Object.keys(node).some((key) => key.startsWith("__svelte")))) frameworks.push({name: "Svelte", version: "", evidence: "rendered Svelte component"});
  const pageScripts = selectAll("script[src]").map((item) => item.src).filter(Boolean);
  const resources = performance.getEntriesByType("resource").filter((item) => item.initiatorType === "script").map((item) => item.name);
  const linkRecords = selectAll("a[href]").map((item) => ({
    url: new URL(item.getAttribute("href"), location.href).href,
    text: String(item.innerText || item.textContent || "").replace(/\s+/g, " ").trim().slice(0, 240)
  }));
  const links = linkRecords.map((item) => item.url);
  const forms = selectAll("form").map((form) => ({
    action: form.action || location.href, method: String(form.method || "GET").toUpperCase(),
    id: form.id || "", name: form.name || "", class: String(form.className || ""),
    text: String(form.innerText || form.textContent || "").replace(/\s+/g, " ").trim().slice(0, 500),
    fields: Array.from(form.elements || []).map((field) => ({
      name: String(field.name || field.id || ""), type: String(field.type || field.tagName || "").toLowerCase(),
      required: Boolean(field.required), placeholder: String(field.placeholder || "").slice(0, 160)
    })).filter((field) => field.name).slice(0, 80)
  }));
  const hardDangerous = /(delete|remove|destroy|drop|erase|logout|sign\s*out|pay|purchase|checkout|reset|删除|移除|销毁|退出|注销|支付|购买|重置)/i;
  const captureOnlyLabel = /(submit|save|create|add\s+new|send|publish|upload|import|confirm|提交|保存|新建|创建|新增|发送|发布|上传|导入|确认)/i;
  const valuable = /(admin|dashboard|account|profile|user|role|permission|member|order|invoice|payment|config|setting|system|audit|log|report|search|query|detail|list|api|graphql|file|document|message|notification|管理|控制台|账户|用户|角色|权限|成员|订单|发票|支付|配置|设置|系统|审计|日志|报表|查询|搜索|详情|列表|接口|文件|消息|通知)/i;
  const candidateNodes = selectAll("a[href],button,[role=button],[role=tab],[role=menuitem],summary,input[type=button],input[type=submit]");
  const candidates = candidateNodes.map((element, index) => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    const text = String(element.innerText || element.textContent || element.getAttribute("aria-label") || element.getAttribute("title") || element.value || "").replace(/\s+/g, " ").trim().slice(0, 240);
    const href = element.matches("a[href]") ? new URL(element.getAttribute("href"), location.href).href : "";
    const role = String(element.getAttribute("role") || element.tagName || "").toLowerCase();
    const type = String(element.getAttribute("type") || "").toLowerCase();
    const formMethod = String(element.form?.method || "").toLowerCase();
    const visible = rect.width > 1 && rect.height > 1 && style.visibility !== "hidden" && style.display !== "none";
    const unsafeSubmit = type === "submit" && formMethod !== "get";
    const captureOnly = unsafeSubmit || captureOnlyLabel.test(text);
    const blocked = !visible || Boolean(element.disabled) || hardDangerous.test(text);
    let score = href ? 45 : 30;
    if (valuable.test(text + " " + href)) score += 35;
    if (role === "tab" || role === "menuitem" || element.tagName === "SUMMARY") score += 15;
    if (captureOnly) score += 20;
    if (blocked) score = -100;
    const id = "oviraptor-" + index;
    element.setAttribute("data-oviraptor-action-id", id);
    return {
      id, text, href, role, type, formMethod, score, blocked, captureOnly,
      blockReason: blocked ? "destructive_or_unusable" : captureOnly ? "capture_request_without_forwarding" : "",
    };
  }).filter((item) => item.text || item.href).sort((a, b) => b.score - a.score).slice(0, 120);
  const bodyText = String(document.body?.innerText || "").replace(/\s+/g, " ").trim();
  const highValueLabels = [...new Set(candidates.filter((item) => item.score >= 65).map((item) => item.text).filter(Boolean))].slice(0, 30);
  return {
    url: location.href, title: document.title || "", frameworks, routes,
    scripts: [...pageScripts, ...resources], requests: state.requests || [], navigations: state.navigations || [],
    links, linkRecords, forms, candidates, highValueLabels,
    bodyPreview: bodyText.slice(0, 5000), domNodes: nodes.length
  };
})();`;

  async function evaluateSnapshot() {
    const evaluated = await send("Runtime.evaluate", {
      expression: evaluation,
      returnByValue: true,
      awaitPromise: true,
    }, sessionId);
    return evaluated.result?.value || {};
  }

  async function navigate(url) {
    await send("Page.navigate", { url }, sessionId);
    await sleep(Math.min(3200, Math.max(900, settleMs * 2)));
  }

  try {
    const version = await send("Browser.getVersion");
    const target = await send("Target.createTarget", { url: "about:blank" });
    const attached = await send("Target.attachToTarget", { targetId: target.targetId, flatten: true });
    sessionId = attached.sessionId;
    await Promise.all([
      send("Page.enable", {}, sessionId),
      send("Runtime.enable", {}, sessionId),
      send("Network.enable", { maxPostDataSize: 12000 }, sessionId),
      send("DOMStorage.enable", {}, sessionId),
    ]);
    const targetOrigin = new URL(targetUrl).origin;
    const targetHost = normalizedHost(targetUrl);
    const sessionScopes = Array.isArray(authSession.scopeHosts) ? authSession.scopeHosts : [];
    const authApplied = Boolean(authSession.id) && hostInScope(targetHost, sessionScopes);
    if (authApplied) {
      const replayHeaders = Object.fromEntries(Object.entries(authSession.headers || {})
        .filter(([name, value]) => value != null && String(value) && !/^(?:host|cookie|content-length|origin|referer|sec-|accept|user-agent)/i.test(String(name)))
        .map(([name, value]) => [String(name), String(value).slice(0, 8192)]));
      if (Object.keys(replayHeaders).length) {
        await send("Network.setExtraHTTPHeaders", { headers: replayHeaders }, sessionId).catch(() => {});
      }
      const replayCookies = (Array.isArray(authSession.cookies) ? authSession.cookies : [])
        .filter((cookie) => cookie && cookie.name && cookie.value)
        .map((cookie) => ({
          name: String(cookie.name), value: String(cookie.value),
          domain: String(cookie.domain || targetHost), path: String(cookie.path || "/"),
          secure: Boolean(cookie.secure), httpOnly: Boolean(cookie.httpOnly),
          ...(cookie.sameSite && ["Strict", "Lax", "None"].includes(String(cookie.sameSite)) ? { sameSite: String(cookie.sameSite) } : {}),
        }));
      if (replayCookies.length) await send("Network.setCookies", { cookies: replayCookies }, sessionId).catch(() => {});
      for (const [key, value] of Object.entries(authSession.localStorage || {}).slice(0, 96)) {
        await send("DOMStorage.setDOMStorageItem", {
          storageId: { securityOrigin: targetOrigin, isLocalStorage: true }, key: String(key), value: String(value),
        }, sessionId).catch(() => {});
      }
      const localStorageSeed = JSON.stringify(authSession.localStorage || {});
      const sessionStorageSeed = JSON.stringify(authSession.sessionStorage || {});
      const originSeed = JSON.stringify(targetOrigin);
      await send("Page.addScriptToEvaluateOnNewDocument", { source: `(() => {
        if (location.origin !== ${originSeed}) return;
        const local = ${localStorageSeed}; const session = ${sessionStorageSeed};
        for (const [key, value] of Object.entries(local)) try { localStorage.setItem(key, String(value)); } catch {}
        for (const [key, value] of Object.entries(session)) try { sessionStorage.setItem(key, String(value)); } catch {}
      })();` }, sessionId);
    }
    await send("Network.setCacheDisabled", { cacheDisabled: true }, sessionId).catch(() => {});
    await send("Page.setDownloadBehavior", { behavior: "deny" }, sessionId).catch(() => {});
    await send("Fetch.enable", {
      patterns: [{ urlPattern: "*", requestStage: "Request" }],
    }, sessionId).catch(() => {});
    await send("Page.addScriptToEvaluateOnNewDocument", { source: initScript }, sessionId);

    const origin = new URL(targetUrl).origin;
    const deadline = Date.now() + explorationTimeoutMs;
    const entryUrl = sameOriginUrl(targetUrl, origin);
    const entryKey = explorationKey(entryUrl, origin);
    const queue = [{ url: entryUrl, key: entryKey, depth: 0, source: "entry", priority: 1000 }];
    const queued = new Set([entryKey]);
    const visited = new Set();
    const queuedFamilies = new Map();
    const clicked = new Set();
    const stateSignatures = new Set();
    const states = [];
    const actions = [];
    const allFrameworks = [];
    const allRoutes = [];
    const allLinks = [];
    const allLinkRecords = [];
    const allForms = [];
    const allNavigations = [];
    let deduplicatedStateCount = 0;
    let lowValueStateSkipped = 0;
    let stopReason = "no_more_actions";

    function enqueue(value, depth, source) {
      const normalized = sameOriginUrl(value, origin);
      if (!normalized || depth > maxDepth) return;
      if (/\.(?:css|m?js|map|png|jpe?g|gif|svg|webp|avif|woff2?|ttf|eot|ico|pdf|zip)(?:[?#]|$)/i.test(normalized)) return;
      if (/[:*{}]/.test(new URL(normalized).pathname)) return;
      const key = explorationKey(normalized, origin);
      if (!key || queued.has(key) || visited.has(key)) {
        deduplicatedStateCount += 1;
        return;
      }
      const parsed = new URL(normalized);
      const noiseOnlyQuery = Boolean(parsed.search)
        && [...parsed.searchParams.keys()].every((name) => navigationNoiseKeys.test(name));
      if (lowValueNavigation.test(parsed.pathname) || noiseOnlyQuery) {
        lowValueStateSkipped += 1;
        return;
      }
      const family = documentationNavigation.test(parsed.pathname) ? "documentation" : "";
      if (family && Number(queuedFamilies.get(family) || 0) >= 3) {
        lowValueStateSkipped += 1;
        return;
      }
      const priority = explorationPriority(normalized, source, depth);
      if (priority <= 0) {
        lowValueStateSkipped += 1;
        return;
      }
      queued.add(key);
      if (family) queuedFamilies.set(family, Number(queuedFamilies.get(family) || 0) + 1);
      queue.push({ url: normalized, key, depth, source, priority });
      queue.sort((left, right) => right.priority - left.priority || left.depth - right.depth);
    }

    function snapshotSignature(snapshot) {
      let pathKey = "/";
      try { pathKey = new URL(String(snapshot.url || targetUrl)).pathname.replace(/\/{2,}/g, "/"); } catch {}
      const formsKey = (snapshot.forms || []).map((form) => {
        let actionPath = "/";
        try { actionPath = new URL(String(form.action || ""), origin).pathname; } catch {}
        const fields = (form.fields || []).map((field) => `${field.type}:${field.name}`).sort().join(",");
        return `${form.method}:${actionPath}:${fields}`;
      }).sort();
      const controlsKey = (snapshot.candidates || []).map((item) => {
        let hrefPath = "";
        try { hrefPath = item.href ? new URL(item.href).pathname : ""; } catch {}
        return `${item.role}:${item.type}:${item.formMethod}:${hrefPath}:${Boolean(item.captureOnly)}`;
      }).sort().slice(0, 80);
      return JSON.stringify([pathKey, formsKey, controlsKey, Number(snapshot.domNodes || 0)]);
    }

    while (queue.length && states.length < maxStates && Date.now() < deadline) {
      const entry = queue.shift();
      if (!entry || visited.has(entry.key)) continue;
      visited.add(entry.key);
      const stateId = `state-${states.length + 1}`;
      activeContext = { actionId: "navigation", stateId, feature: entry.source };
      await navigate(entry.url);
      let snapshot = await evaluateSnapshot();
      const resolvedKey = explorationKey(String(snapshot.url || entry.url), origin);
      if (resolvedKey && resolvedKey !== entry.key && visited.has(resolvedKey)) {
        deduplicatedStateCount += 1;
        continue;
      }
      if (resolvedKey) {
        visited.add(resolvedKey);
        queued.add(resolvedKey);
      }
      const signature = snapshotSignature(snapshot);
      if (stateSignatures.has(signature)) {
        deduplicatedStateCount += 1;
        continue;
      }
      stateSignatures.add(signature);
      const state = {
        id: stateId, url: String(snapshot.url || entry.url), title: String(snapshot.title || ""),
        depth: entry.depth, discoveredFrom: entry.source, domNodes: Number(snapshot.domNodes || 0),
        bodyPreview: String(snapshot.bodyPreview || ""), highValueLabels: snapshot.highValueLabels || [],
        forms: snapshot.forms || [], candidates: snapshot.candidates || [],
        requestStart: requests.length,
      };
      states.push(state);
      allFrameworks.push(...(snapshot.frameworks || []));
      allRoutes.push(...(snapshot.routes || []));
      scripts.push(...(snapshot.scripts || []));
      allLinks.push(...(snapshot.links || []));
      allLinkRecords.push(...(snapshot.linkRecords || []));
      allForms.push(...(snapshot.forms || []));
      allNavigations.push(...(snapshot.navigations || []));
      if (confirmedWaf(requests, snapshot.bodyPreview || "")) {
        stopReason = "confirmed_waf_or_challenge";
        break;
      }

      for (const link of snapshot.links || []) enqueue(link, entry.depth + 1, `link:${stateId}`);
      for (const route of snapshot.routes || []) {
        const routePath = String(route.path || "");
        if (!routePath || /[:*{}]/.test(routePath)) continue;
        const routeUrl = new URL(routePath, origin).href;
        enqueue(routeUrl, entry.depth + 1, `router:${stateId}`);
        if (String(snapshot.url || "").includes("#/")) enqueue(`${origin}/#${routePath}`, entry.depth + 1, `hash-router:${stateId}`);
      }

      let stateActionCount = 0;
      while (stateActionCount < 8 && actions.length < maxActions && Date.now() < deadline) {
        // Re-read candidates after every click. Tabs, menus and dialogs often
        // reveal the valuable control only after an earlier control changed the DOM.
        const candidate = (snapshot.candidates || []).find((item) => {
          if (item.blocked || item.href) return false;
          const key = `${state.url}|${item.role}|${item.text}`;
          return !clicked.has(key);
        });
        if (!candidate) break;
        const clickKey = `${state.url}|${candidate.role}|${candidate.text}`;
        clicked.add(clickKey);
        stateActionCount += 1;
        const actionId = `action-${actions.length + 1}`;
        activeContext = {
          actionId,
          stateId,
          feature: String(candidate.text || candidate.role || "control"),
          captureOnly: Boolean(candidate.captureOnly),
        };
        const requestStart = requests.length;
        const blockedStart = blockedRequests.length;
        const beforeUrl = String(snapshot.url || entry.url);
        const actionStarted = Date.now();
        let outcome = "triggered";
        let error = "";
        try {
          const clickedResult = await send("Runtime.evaluate", {
            expression: `(() => {
              const roots = [document];
              for (let index = 0; index < roots.length; index += 1) {
                const root = roots[index];
                const item = root.querySelector?.('[data-oviraptor-action-id=${JSON.stringify(candidate.id).slice(1, -1)}]');
                if (item) { item.click(); return true; }
                for (const element of Array.from(root.querySelectorAll?.('*') || [])) {
                  if (element.shadowRoot && !roots.includes(element.shadowRoot)) roots.push(element.shadowRoot);
                  if (element.tagName === 'IFRAME') { try { if (element.contentDocument && !roots.includes(element.contentDocument)) roots.push(element.contentDocument); } catch {} }
                }
              }
              return false;
            })()`,
            returnByValue: true,
          }, sessionId);
          if (!clickedResult.result?.value) outcome = "element_missing";
          await sleep(settleMs);
          snapshot = await evaluateSnapshot();
        } catch (actionError) {
          outcome = "error";
          error = String(actionError?.message || actionError).slice(0, 500);
        }
        const afterUrl = String(snapshot.url || beforeUrl);
        const action = {
          id: actionId, stateId, label: String(candidate.text || ""), role: String(candidate.role || ""),
          score: Number(candidate.score || 0), captureOnly: Boolean(candidate.captureOnly), outcome, error, beforeUrl, afterUrl,
          stateChanged: afterUrl !== beforeUrl || requests.length > requestStart,
          requestCount: requests.length - requestStart,
          blockedRequestCount: blockedRequests.length - blockedStart,
          durationMs: Date.now() - actionStarted,
        };
        actions.push(action);
        allLinks.push(...(snapshot.links || []));
        allLinkRecords.push(...(snapshot.linkRecords || []));
        allForms.push(...(snapshot.forms || []));
        allRoutes.push(...(snapshot.routes || []));
        scripts.push(...(snapshot.scripts || []));
        if (afterUrl !== beforeUrl) {
          enqueue(afterUrl, entry.depth + 1, `action:${actionId}`);
          break;
        }
      }
      state.requestEnd = requests.length;
    }

    if (Date.now() >= deadline) stopReason = "exploration_deadline";
    else if (actions.length >= maxActions) stopReason = "action_budget_reached";
    else if (states.length >= maxStates && queue.length) stopReason = "state_budget_reached";
    else if (queue.length === 0) stopReason = "no_more_valuable_states";

    const runtimeHookRequests = [];
    try {
      const finalSnapshot = await evaluateSnapshot();
      runtimeHookRequests.push(...(finalSnapshot.requests || []));
      allNavigations.push(...(finalSnapshot.navigations || []));
    } catch {}
    for (const request of runtimeHookRequests) {
      if (requests.length >= maxRequests) break;
      const resolvedUrl = (() => {
        try { return new URL(String(request.url || ""), origin).href; } catch { return String(request.url || ""); }
      })();
      const method = String(request.method || "GET").toUpperCase();
      if (requests.some((item) => String(item.method || "GET").toUpperCase() === method && item.url === resolvedUrl)) continue;
      requests.push({
        url: resolvedUrl, method,
        resourceType: String(request.resourceType || ""), source: "browser-runtime-hook",
        queryKeys: queryKeys(request.url), bodyKeys: [], actionId: "runtime-hook", stateId: "", feature: "",
      });
    }

    if (responseReads.length) {
      await Promise.race([
        Promise.allSettled(responseReads),
        sleep(Math.min(2500, pageTimeoutMs)),
      ]);
    }

    const groupedRequests = new Map();
    for (const item of requests) {
      const key = `${item.method}|${item.url}|${item.postData || ""}`;
      const current = groupedRequests.get(key);
      if (!current) {
        groupedRequests.set(key, { ...item });
        continue;
      }
      for (const [field, value] of Object.entries(item)) {
        const existing = current[field];
        const missing = existing == null || existing === "" || (Array.isArray(existing) && existing.length === 0)
          || (typeof existing === "object" && !Array.isArray(existing) && Object.keys(existing).length === 0);
        if (missing && value != null && value !== "") current[field] = value;
      }
      for (const field of ["headers", "effectiveRequestHeaders", "responseHeaders", "effectiveResponseHeaders"]) {
        current[field] = mergedHeaders(current[field] || {}, item[field] || {});
      }
      for (const field of ["headerNames", "effectiveRequestHeaderNames", "extraInfoRequestHeaderNames", "extraRequestHeaderNames", "responseHeaderNames", "effectiveResponseHeaderNames", "extraResponseHeaderNames", "queryKeys", "bodyKeys", "responseKeys"]) {
        current[field] = dedupe([...(current[field] || []), ...(item[field] || [])], String).slice(0, 120);
      }
    }
    const finalRequests = [...groupedRequests.values()].map(({ requestId, ...item }) => item);
    const wafDetected = confirmedWaf(finalRequests, states.map((state) => state.bodyPreview || "").join(" "));
    const permissionBoundaryCount = finalRequests.filter((item) => [401, 403].includes(Number(item.status || 0))).length;
    const successfulBusinessRequest = finalRequests.some((item) => ["xhr", "fetch"].includes(String(item.resourceType || "").toLowerCase()) && Number(item.status || 0) >= 200 && Number(item.status || 0) < 400);
    const finalStateUrl = String(states.at(-1)?.url || targetUrl);
    const clearSessionInvalid = Boolean(authApplied && !successfulBusinessRequest && loginLike(finalStateUrl) && !loginLike(targetUrl));
    const authSessionValidation = {
      applied: authApplied,
      valid: Boolean(authApplied && !clearSessionInvalid && (successfulBusinessRequest || !loginLike(finalStateUrl))),
      clearSessionInvalid,
      wafDetected,
      permissionBoundaryCount,
      successfulBusinessRequest,
      finalUrl: finalStateUrl,
      reason: !authApplied ? "target_outside_session_scope" : clearSessionInvalid ? "redirected_to_login" : wafDetected ? "confirmed_waf_or_challenge" : permissionBoundaryCount ? "authorization_boundaries_observed" : "session_active",
    };
    const features = states.map((state) => ({
      stateId: state.id, url: state.url, title: state.title, depth: state.depth,
      highValueLabels: state.highValueLabels, formCount: state.forms.length,
      fieldNames: dedupe(state.forms.flatMap((form) => (form.fields || []).map((field) => field.name)), String).slice(0, 80),
      interactiveCount: state.candidates.filter((candidate) => !candidate.blocked).length,
      bodyPreview: state.bodyPreview,
    }));
    const result = {
      available: true,
      browser: String(version.product || executable),
      frameworks: dedupe(allFrameworks, (item) => `${item.name}|${item.version}|${item.evidence}`),
      routes: dedupe(allRoutes, (item) => `${item.path}|${item.name}`),
      scripts: dedupe(scripts, String),
      requests: finalRequests,
      links: dedupe(allLinks, String),
      linkRecords: dedupe(allLinkRecords, (item) => `${item.url}|${item.text}`),
      forms: dedupe(allForms, (item) => `${item.method}|${item.action}|${(item.fields || []).map((field) => field.name).join(",")}`),
      navigations: dedupe(allNavigations, String),
      states,
      actions,
      features,
      blockedRequests: dedupe(blockedRequests, (item) => `${item.method}|${item.url}|${item.postData || ""}`),
      authSessionValidation,
      coverage: {
        stateCount: states.length,
        actionCount: actions.length,
        requestCount: finalRequests.length,
        xhrFetchCount: finalRequests.filter((item) => ["xhr", "fetch"].includes(String(item.resourceType || "").toLowerCase())).length,
        responseBodyCount: finalRequests.filter((item) => item.responsePreview).length,
        blockedMutationCount: blockedRequests.length,
        queuedStateCount: queue.length,
        deduplicatedStateCount,
        lowValueStateSkipped,
        maxStates,
        maxActions,
        maxDepth,
      },
      stopReason,
      durationMs: Date.now() - startedAt,
      errors: [],
    };
    process.stdout.write(JSON.stringify(result));
  } catch (error) {
    process.stdout.write(JSON.stringify({
      ...empty,
      browser: executable,
      durationMs: Date.now() - startedAt,
      errors: [String(error?.message || error), ...stderr].filter(Boolean).map((item) => String(item).slice(0, 1000)),
    }));
  } finally {
    for (const waiter of pending.values()) {
      clearTimeout(waiter.timer);
      waiter.reject(new Error("browser_closed"));
    }
    pending.clear();
    child.kill("SIGTERM");
    try {
      fs.rmSync(profileDir, { recursive: true, force: true });
    } catch {}
  }
}

main().catch((error) => {
  process.stdout.write(JSON.stringify({ ...empty, errors: [String(error?.message || error)] }));
});
