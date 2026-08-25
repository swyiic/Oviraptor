export type HttpRequestMessage = {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
};

export type HttpResponseMessage = {
  status: number | null;
  statusText?: string;
  headers?: Record<string, string>;
  body?: string;
};

function headerEntries(headers: unknown): Array<[string, string]> {
  if (!headers || typeof headers !== "object" || Array.isArray(headers)) return [];
  return Object.entries(headers as Record<string, unknown>)
    .filter(([name, value]) => name.trim() && value != null)
    .map(([name, value]) => [name.trim(), String(value)]);
}

function requestHeaderEntries(headers: unknown): Array<[string, string]> {
  const normalized = new Map<string, [string, string]>();
  for (const [name, value] of headerEntries(headers)) {
    const lower = name.toLowerCase();
    // CDP exposes HTTP/2 pseudo headers alongside the ordinary request
    // headers. They are transport metadata, not valid HTTP/1 header lines.
    if (name.startsWith(":") || ["content-length", "transfer-encoding", "connection"].includes(lower)) continue;
    const existing = normalized.get(lower);
    if (existing && lower === "cookie" && !existing[1].split(/;\s*/).includes(value)) {
      normalized.set(lower, [existing[0], `${existing[1]}; ${value}`]);
    } else if (!existing) {
      normalized.set(lower, [name, value]);
    }
  }
  return [...normalized.values()];
}

function requestTarget(url: string) {
  try {
    const parsed = new URL(url);
    return `${parsed.pathname || "/"}${parsed.search}`;
  } catch {
    return url || "/";
  }
}

export function requestHost(url: string) {
  try { return new URL(url).host; } catch { return ""; }
}

export function buildRawHttpRequest(message: HttpRequestMessage) {
  const headers = requestHeaderEntries(message.headers);
  const names = new Set(headers.map(([name]) => name.toLowerCase()));
  const lines = [`${message.method.toUpperCase()} ${requestTarget(message.url)} HTTP/1.1`];
  const host = requestHost(message.url);
  if (host && !names.has("host")) lines.push(`Host: ${host}`);
  for (const [name, value] of headers) {
    if (["content-length", "transfer-encoding", "connection"].includes(name.toLowerCase())) continue;
    lines.push(`${name}: ${value}`);
  }
  if (message.body && !names.has("content-length")) {
    lines.push(`Content-Length: ${new TextEncoder().encode(message.body).length}`);
  }
  return `${lines.join("\r\n")}\r\n\r\n${message.body || ""}`;
}

export function parseRawHttpRequest(raw: string, fallbackUrl = ""): HttpRequestMessage {
  const normalized = raw.replace(/\r\n/g, "\n");
  const separator = normalized.indexOf("\n\n");
  const head = separator >= 0 ? normalized.slice(0, separator) : normalized;
  const body = separator >= 0 ? normalized.slice(separator + 2) : "";
  const lines = head.split("\n");
  const requestLine = (lines.shift() || "").trim();
  const match = requestLine.match(/^([A-Za-z]+)\s+(\S+)(?:\s+HTTP\/\d(?:\.\d)?)?$/);
  if (!match) throw new Error("第一行应为：METHOD /path HTTP/1.1");
  const headers: Record<string, string> = {};
  let lastHeader = "";
  for (const line of lines) {
    if (!line.trim()) continue;
    // Accept old saved messages that still contain CDP HTTP/2 pseudo headers.
    // The URL and method are already reconstructed from the request line.
    if (/^:[a-z0-9_-]+\s*:/i.test(line.trim())) continue;
    if (/^[ \t]/.test(line) && lastHeader) {
      headers[lastHeader] = `${headers[lastHeader]} ${line.trim()}`;
      continue;
    }
    const colon = line.indexOf(":");
    if (colon <= 0) throw new Error(`请求头格式无效：${line}`);
    const name = line.slice(0, colon).trim();
    const value = line.slice(colon + 1).trim();
    if (!name) throw new Error("请求头名称不能为空");
    const existingName = Object.keys(headers).find((item) => item.toLowerCase() === name.toLowerCase());
    const canonicalName = existingName || name;
    if (!existingName) headers[canonicalName] = value;
    else if (name.toLowerCase() === "cookie" && !headers[canonicalName].split(/;\s*/).includes(value)) {
      headers[canonicalName] = `${headers[canonicalName]}; ${value}`;
    }
    lastHeader = canonicalName;
  }
  const target = match[2];
  let url = target;
  if (!/^https?:\/\//i.test(target)) {
    const host = Object.entries(headers).find(([name]) => name.toLowerCase() === "host")?.[1];
    const scheme = (() => { try { return new URL(fallbackUrl).protocol || "https:"; } catch { return "https:"; } })();
    if (host) url = `${scheme}//${host}${target.startsWith("/") ? target : `/${target}`}`;
    else {
      try { url = new URL(target, fallbackUrl).href; } catch { throw new Error("相对路径请求必须包含 Host 请求头"); }
    }
  }
  const transportManaged = new Set(["host", "content-length", "transfer-encoding", "connection"]);
  const sendHeaders = Object.fromEntries(Object.entries(headers).filter(([name]) => !name.startsWith(":") && !transportManaged.has(name.toLowerCase())));
  return { method: match[1].toUpperCase(), url, headers: sendHeaders, body };
}

export function buildRawHttpResponse(message: HttpResponseMessage) {
  if (message.status == null) return "未捕获同一接口响应";
  const lines = [`HTTP/1.1 ${message.status}${message.statusText ? ` ${message.statusText}` : ""}`];
  for (const [name, value] of headerEntries(message.headers)) lines.push(`${name}: ${value}`);
  return `${lines.join("\r\n")}\r\n\r\n${message.body || ""}`;
}

export function prettyHttpBody(value: string) {
  if (!value) return "";
  try { return JSON.stringify(JSON.parse(value), null, 2); } catch { return value; }
}
