#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

let babel;
try {
  babel = require(path.join(__dirname, "babel-parser.cjs"));
} catch {
  babel = require("@babel/parser");
}

const input = JSON.parse(fs.readFileSync(0, "utf8"));
const source = String(input.source || "");
const sourceUrl = String(input.url || "");
const bindings = new Map();
const clients = new Map([["axios", ""]]);
const clientHeaders = new Map();
const headerContainers = new Set();
const imports = [];
const apis = [];
const routes = [];
const baseUrls = [];
const stringEvidence = {
  baseUrls: [],
  apiPrefixes: [],
  businessPaths: [],
  storageReferences: [],
};
const codeSlices = [];
const headerEvidence = [];
const parseErrors = [];

function children(node) {
  if (!node || typeof node !== "object") return [];
  const result = [];
  for (const [key, value] of Object.entries(node)) {
    if (["loc", "start", "end", "extra", "errors", "comments", "tokens"].includes(key)) continue;
    if (Array.isArray(value)) {
      for (const item of value) if (item && typeof item.type === "string") result.push(item);
    } else if (value && typeof value.type === "string") result.push(value);
  }
  return result;
}

function walk(node, visit, parent = null, ancestors = []) {
  if (!node || typeof node.type !== "string") return;
  visit(node, parent, ancestors);
  for (const child of children(node)) walk(child, visit, node, [...ancestors, node]);
}

function propertyName(node) {
  if (!node) return "";
  if (node.type === "Identifier" || node.type === "PrivateName") return node.name || "";
  if (node.type === "StringLiteral" || node.type === "NumericLiteral") return String(node.value);
  return "";
}

function memberName(node) {
  if (!node) return "";
  if (node.type === "Identifier") return node.name;
  if (node.type === "ThisExpression") return "this";
  if (node.type === "MemberExpression" || node.type === "OptionalMemberExpression") {
    const object = memberName(node.object);
    const property = propertyName(node.property);
    return object && property ? `${object}.${property}` : property || object;
  }
  if (node.type === "SequenceExpression" && node.expressions.length) return memberName(node.expressions.at(-1));
  return "";
}

function evaluate(node, seen = new Set(), depth = 0) {
  if (!node || depth > 14) return null;
  if (["StringLiteral", "NumericLiteral", "BooleanLiteral"].includes(node.type)) {
    return { value: String(node.value), dynamic: false };
  }
  if (node.type === "NullLiteral") return { value: "", dynamic: false };
  if (node.type === "Identifier") {
    if (seen.has(node.name)) return { value: `<${node.name}>`, dynamic: true };
    const bound = bindings.get(node.name);
    if (!bound) return { value: `<${node.name}>`, dynamic: true };
    return evaluate(bound, new Set([...seen, node.name]), depth + 1);
  }
  if (node.type === "TemplateLiteral") {
    let value = "";
    let dynamic = false;
    for (let index = 0; index < node.quasis.length; index += 1) {
      value += node.quasis[index].value.cooked ?? node.quasis[index].value.raw ?? "";
      if (index < node.expressions.length) {
        const part = evaluate(node.expressions[index], seen, depth + 1);
        value += part?.value ?? "<expr>";
        dynamic ||= !part || part.dynamic;
      }
    }
    return { value, dynamic };
  }
  if (node.type === "BinaryExpression" && node.operator === "+") {
    const left = evaluate(node.left, seen, depth + 1);
    const right = evaluate(node.right, seen, depth + 1);
    if (!left || !right) return null;
    return { value: left.value + right.value, dynamic: left.dynamic || right.dynamic };
  }
  if (node.type === "ConditionalExpression") {
    const yes = evaluate(node.consequent, seen, depth + 1);
    const no = evaluate(node.alternate, seen, depth + 1);
    if (yes && no && yes.value === no.value) return { value: yes.value, dynamic: yes.dynamic || no.dynamic };
    return yes || no;
  }
  if (node.type === "LogicalExpression") return evaluate(node.right, seen, depth + 1) || evaluate(node.left, seen, depth + 1);
  if (node.type === "ParenthesizedExpression" || node.type === "TSAsExpression") return evaluate(node.expression, seen, depth + 1);
  if (node.type === "MemberExpression" || node.type === "OptionalMemberExpression") {
    const object = node.object.type === "Identifier" ? bindings.get(node.object.name) : node.object;
    const key = propertyName(node.property);
    if (object?.type === "ObjectExpression") {
      const property = object.properties.find((item) => item.type === "ObjectProperty" && propertyName(item.key) === key);
      if (property) return evaluate(property.value, seen, depth + 1);
    }
    return { value: `<${memberName(node) || "member"}>`, dynamic: true };
  }
  if (node.type === "CallExpression" || node.type === "OptionalCallExpression") {
    const name = memberName(node.callee);
    if (["String", "decodeURI", "decodeURIComponent"].includes(name) && node.arguments.length) return evaluate(node.arguments[0], seen, depth + 1);
    if (name.endsWith(".join") && node.callee?.object) {
      const array = unwrap(node.callee.object, seen, depth + 1);
      if (array?.type === "ArrayExpression") {
        const separator = evaluate(node.arguments[0], seen, depth + 1)?.value ?? ",";
        const parts = (array.elements || []).map((item) => evaluate(item, seen, depth + 1));
        if (parts.every(Boolean)) {
          return {
            value: parts.map((item) => item.value).join(separator),
            dynamic: parts.some((item) => item.dynamic),
          };
        }
      }
    }
  }
  return null;
}

function objectProperty(node, name) {
  if (!node || node.type !== "ObjectExpression") return null;
  const item = node.properties.find((property) => property.type === "ObjectProperty" && propertyName(property.key).toLowerCase() === name.toLowerCase());
  return item?.value || null;
}

function objectKeys(node) {
  if (!node || node.type !== "ObjectExpression") return [];
  return node.properties.filter((item) => item.type === "ObjectProperty").map((item) => propertyName(item.key)).filter(Boolean);
}

function pushHeaderEvidence(name, value, node, sourceKind, dynamic = false) {
  const normalizedName = String(name || "").trim();
  if (!normalizedName || normalizedName.startsWith("<") || normalizedName.length > 160) return null;
  const record = {
    name: normalizedName,
    value: String(value || "").slice(0, 1200),
    dynamic: Boolean(dynamic),
    sensitive: /^(?:authorization|proxy-authorization|cookie|set-cookie|x-api-key|api-key|x-auth-token)$/i.test(normalizedName),
    sourceKind: String(sourceKind || "javascript-declared"),
    source: sourceUrl,
    line: Number(node?.loc?.start?.line || 0),
    evidence: evidence(node),
  };
  if (!headerEvidence.some((item) => item.name.toLowerCase() === record.name.toLowerCase() && item.value === record.value && item.sourceKind === record.sourceKind)) {
    headerEvidence.push(record);
  }
  return record;
}

function extractHeaderEntries(inputNode, evidenceNode, sourceKind = "request-config", publish = true) {
  let node = unwrap(inputNode);
  if (node?.type === "NewExpression" && memberName(node.callee) === "Headers") node = unwrap(node.arguments?.[0]);
  if (!node || node.type !== "ObjectExpression") return [];
  const result = [];
  for (const property of node.properties || []) {
    if (property.type !== "ObjectProperty") continue;
    const evaluatedName = property.computed ? evaluate(property.key) : null;
    const name = propertyName(property.key) || evaluatedName?.value || "";
    const evaluatedValue = evaluate(property.value);
    if (!name) continue;
    const record = publish
      ? pushHeaderEvidence(name, evaluatedValue?.value || "<dynamic>", evidenceNode || property, sourceKind, !evaluatedValue || evaluatedValue.dynamic)
      : {
          name,
          value: String(evaluatedValue?.value || "<dynamic>").slice(0, 1200),
          dynamic: !evaluatedValue || evaluatedValue.dynamic,
          sensitive: /^(?:authorization|proxy-authorization|cookie|set-cookie|x-api-key|api-key|x-auth-token)$/i.test(name),
          sourceKind,
          source: sourceUrl,
          line: Number((evidenceNode || property)?.loc?.start?.line || 0),
        };
    if (record) result.push(record);
  }
  return result;
}

function evidence(node) {
  const start = Math.max(0, Number(node.start || 0) - 80);
  const end = Math.min(source.length, Number(node.end || start) + 160);
  return source.slice(start, end).replace(/[\r\n]+/g, " ").slice(0, 320);
}

const API_MARKER = /(?:^|\/)(?:api|apis|rest|restapi|openapi|gateway|gw|backend|service|services|graphql|rpc)(?:\/|$)/i;
const BUSINESS_MARKER = /(?:^|[/_.-])(?:admin|auth|login|logout|token|session|user|member|role|permission|account|profile|order|invoice|payment|upload|download|export|import|config|setting|system|audit|report|search|query|list|page|detail|info|create|add|save|update|edit|delete|remove|submit|verify|check)(?:$|[/_.?-])/i;
const STATIC_SUFFIX = /\.(?:avif|bmp|css|eot|gif|ico|jpe?g|js|map|mp3|mp4|pdf|png|svg|ttf|webp|woff2?)(?:[?#]|$)/i;

function normalizedEvidenceValue(raw) {
  return String(raw || "").replace(/\\\//g, "/").trim();
}

function pushStringEvidence(kind, raw, node, label = "", confidence = "medium") {
  const value = normalizedEvidenceValue(raw);
  if (!value || value.length > 1000 || /^(?:data:|javascript:|mailto:|tel:)/i.test(value) || STATIC_SUFFIX.test(value)) return;
  const bucket = stringEvidence[kind];
  if (!Array.isArray(bucket) || bucket.length >= 120 || bucket.some((item) => item.value === value && item.label === label)) return;
  bucket.push({
    value,
    source: sourceUrl,
    line: Number(node?.loc?.start?.line || 0),
    label: String(label || "").slice(0, 120),
    confidence,
    evidence: evidence(node),
  });
}

function collectNamedStringEvidence(node, name, value) {
  const label = String(name || "");
  const text = normalizedEvidenceValue(value);
  if (!text || text.includes("<expr>")) return;
  if (/^https?:\/\//i.test(text)) {
    pushStringEvidence("baseUrls", text, node, label, "high");
    return;
  }
  if (!text.startsWith("/")) return;
  if (/(?:base.?url|api.?base|api.?url|endpoint|gateway|prefix|context.?path|service.?root)/i.test(label) || API_MARKER.test(text)) {
    pushStringEvidence("apiPrefixes", text, node, label, "high");
  }
  const segments = text.split(/[?#]/, 1)[0].split("/").filter(Boolean);
  if (segments.length >= 2 && (BUSINESS_MARKER.test(text) || (API_MARKER.test(text) && segments.length >= 3))) {
    pushStringEvidence("businessPaths", text, node, label, BUSINESS_MARKER.test(text) ? "high" : "medium");
  }
}

function addCodeSlice(node, ancestors, kind, marker) {
  if (!node || codeSlices.length >= 24) return;
  const boundaries = new Set([
    "FunctionDeclaration", "FunctionExpression", "ArrowFunctionExpression",
    "ObjectMethod", "ClassMethod", "ClassPrivateMethod",
  ]);
  const candidates = [...ancestors, node]
    .filter((item) => boundaries.has(item?.type) && Number.isFinite(item.start) && Number.isFinite(item.end))
    .reverse();
  let boundary = candidates.find((item) => item.end - item.start <= 14_000) || node;
  let start = Math.max(0, Number(boundary.start ?? node.start ?? 0));
  let end = Math.min(source.length, Number(boundary.end ?? node.end ?? start));
  if (end - start > 14_000) {
    const center = Math.floor((Number(node.start || start) + Number(node.end || start)) / 2);
    start = Math.max(0, center - 6_000);
    end = Math.min(source.length, start + 12_000);
    start = Math.max(0, end - 12_000);
  }
  if (end <= start) return;
  const id = crypto.createHash("sha256").update(`${sourceUrl}|${start}|${end}`).digest("hex").slice(0, 16);
  if (codeSlices.some((item) => item.id === id)) return;
  codeSlices.push({
    id,
    source: sourceUrl,
    kind,
    marker: String(marker || "").slice(0, 120),
    start,
    end,
    focusStart: Math.max(0, Number(node.start || start) - start),
    focusEnd: Math.max(0, Number(node.end || start) - start),
    context: source.slice(start, end).replace(/\0/g, ""),
  });
}

function addApi(node, urlNode, method, config, engine = "babel-ast") {
  const evaluated = evaluate(urlNode);
  if (!evaluated || !evaluated.value || evaluated.value.length > 700) return;
  const value = evaluated.value.replace(/\\\//g, "/").trim();
  if (/^(?:data:|javascript:|mailto:|tel:)/i.test(value) || /\.(?:css|js|map|png|jpe?g|gif|svg|woff2?|ttf)(?:[?#]|$)/i.test(value)) return;
  const parameters = new Set();
  for (const key of ["params", "data", "body", "query", "json", "form"]) {
    for (const parameter of objectKeys(objectProperty(config, key))) parameters.add(parameter);
  }
  const clientName = memberName(node.callee).split(".")[0];
  const declaredHeaders = [
    ...(clientHeaders.get(clientName) || []),
    ...extractHeaderEntries(objectProperty(config, "headers"), node, "request-config"),
  ];
  apis.push({
    path: value,
    method: String(method || "UNKNOWN").toUpperCase(),
    parameters: [...parameters],
    source: sourceUrl,
    confidence: evaluated.dynamic ? "medium" : "high",
    extractionEngine: engine,
    dynamic: evaluated.dynamic,
    clientBaseUrl: clients.get(clientName) || "",
    declaredHeaders: dedupe(declaredHeaders, (item) => `${item.name.toLowerCase()}|${item.value}|${item.sourceKind}`),
    evidence: evidence(node),
  });
}

let ast;
try {
  ast = babel.parse(source, {
    sourceType: "unambiguous",
    errorRecovery: true,
    allowAwaitOutsideFunction: true,
    allowReturnOutsideFunction: true,
    plugins: ["jsx", "typescript", "dynamicImport", "decorators-legacy", "classProperties", "optionalChaining", "topLevelAwait"],
  });
  for (const error of ast.errors || []) parseErrors.push(String(error.message || error).slice(0, 300));
} catch (error) {
  process.stdout.write(JSON.stringify({ apis: [], imports: [], routes: [], baseUrls: [], stringEvidence: {baseUrls: [], apiPrefixes: [], businessPaths: [], storageReferences: []}, headerEvidence: [], codeSlices: [], moduleCount: 0, parseErrors: [String(error.message || error)] }));
  process.exit(0);
}

// Multiple passes resolve constants declared after their first use in minified bundles.
for (let pass = 0; pass < 4; pass += 1) {
  walk(ast, (node) => {
    if (node.type === "VariableDeclarator" && node.id?.type === "Identifier" && node.init) bindings.set(node.id.name, node.init);
    if (node.type === "AssignmentExpression" && node.operator === "=" && node.left?.type === "Identifier") bindings.set(node.left.name, node.right);
  });
}

function unwrap(node, seen = new Set(), depth = 0) {
  if (!node || depth > 20) return node;
  if (node.type === "Identifier") {
    if (seen.has(node.name)) return node;
    const bound = bindings.get(node.name);
    return bound ? unwrap(bound, new Set([...seen, node.name]), depth + 1) : node;
  }
  if (["ParenthesizedExpression", "TSAsExpression", "TSSatisfiesExpression", "TypeCastExpression"].includes(node.type)) {
    return unwrap(node.expression, seen, depth + 1);
  }
  return node;
}

function joinRoutePath(parentPath, childPath) {
  const parent = String(parentPath || "").trim();
  const child = String(childPath || "").trim();
  if (!child) return parent || "/";
  if (child === "*" || child === "/*") return `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/*` || "/*";
  if (child.startsWith("/")) return child.replace(/\/{2,}/g, "/");
  return `${parent === "/" ? "" : parent.replace(/\/+$/, "")}/${child}`.replace(/\/{2,}/g, "/") || "/";
}

function addRouteRecord(path, rawPath, parentPath, node, dynamic = false, engine = "babel-ast") {
  if (!path || path.length > 1000) return;
  routes.push({
    path,
    rawPath: rawPath || path,
    parentPath: parentPath || "",
    source: sourceUrl,
    type: "frontend",
    confidence: dynamic ? "medium" : "high",
    extractionEngine: engine,
  });
}

const structuredRouteObjects = new Set();

function extractRouteNode(inputNode, parentPath = "", seen = new Set(), depth = 0) {
  if (!inputNode || depth > 40) return;
  let node = unwrap(inputNode, seen, depth);
  if (!node) return;
  if (node.type === "SpreadElement") {
    extractRouteNode(node.argument, parentPath, seen, depth + 1);
    return;
  }
  if (node.type === "ArrayExpression") {
    for (const element of node.elements || []) extractRouteNode(element, parentPath, seen, depth + 1);
    return;
  }
  if (node.type !== "ObjectExpression") return;
  if (structuredRouteObjects.has(node) && !parentPath) return;
  structuredRouteObjects.add(node);
  const pathNode = objectProperty(node, "path");
  const indexNode = objectProperty(node, "index");
  const childrenNode = objectProperty(node, "children");
  const shape = ["component", "element", "redirect", "loader", "action", "loadChildren", "beforeEnter", "name", "meta", "children"]
    .some((key) => objectProperty(node, key));
  const evaluated = evaluate(pathNode);
  const isIndex = evaluate(indexNode)?.value === "true";
  let currentParent = parentPath;
  if ((evaluated?.value || isIndex) && shape) {
    const rawPath = isIndex && !evaluated?.value ? "" : evaluated.value;
    const fullPath = joinRoutePath(parentPath, rawPath);
    addRouteRecord(fullPath, rawPath || "(index)", parentPath, pathNode || node, Boolean(evaluated?.dynamic));
    currentParent = fullPath;
  }
  if (childrenNode) extractRouteNode(childrenNode, currentParent, seen, depth + 1);
}

function jsxName(node) {
  if (!node) return "";
  if (node.type === "JSXIdentifier") return node.name || "";
  if (node.type === "JSXMemberExpression") return `${jsxName(node.object)}.${jsxName(node.property)}`;
  return "";
}

function jsxAttribute(node, name) {
  return (node?.openingElement?.attributes || []).find(
    (item) => item.type === "JSXAttribute" && jsxName(item.name).toLowerCase() === name.toLowerCase(),
  );
}

function jsxAttributeValue(attribute) {
  if (!attribute) return null;
  if (!attribute.value) return { value: "true", dynamic: false };
  if (attribute.value.type === "StringLiteral") return { value: attribute.value.value, dynamic: false };
  if (attribute.value.type === "JSXExpressionContainer") return evaluate(attribute.value.expression);
  return null;
}

function extractJsxRoutes(node, parentPath = "") {
  if (!node || typeof node !== "object") return;
  if (node.type === "JSXElement") {
    const name = jsxName(node.openingElement?.name).split(".").at(-1);
    if (name === "Route") {
      const pathValue = jsxAttributeValue(jsxAttribute(node, "path"));
      const indexValue = jsxAttributeValue(jsxAttribute(node, "index"));
      const rawPath = indexValue?.value === "true" && !pathValue?.value ? "" : pathValue?.value;
      const fullPath = joinRoutePath(parentPath, rawPath || "");
      addRouteRecord(fullPath, rawPath || "(index)", parentPath, node, Boolean(pathValue?.dynamic), "babel-ast-jsx");
      for (const child of node.children || []) extractJsxRoutes(child, fullPath);
      return;
    }
  }
  for (const child of children(node)) extractJsxRoutes(child, parentPath);
}

// Start from router containers so relative children can be joined to their parents.
walk(ast, (node) => {
  if (node.type === "VariableDeclarator" && node.id?.type === "Identifier" && /(?:^|_)(?:routes?|routerConfig)(?:$|_)/i.test(node.id.name)) {
    extractRouteNode(node.init);
  }
  if (node.type === "ObjectProperty" && ["routes", "children"].includes(propertyName(node.key))) {
    extractRouteNode(node.value);
  }
  if (node.type === "CallExpression" || node.type === "OptionalCallExpression" || node.type === "NewExpression") {
    const callee = memberName(node.callee);
    if (/(?:createRouter|createBrowserRouter|createHashRouter|useRoutes|addRoute|addRoutes|VueRouter)$/i.test(callee)) {
      for (const argument of node.arguments || []) {
        const routesOption = objectProperty(unwrap(argument), "routes");
        extractRouteNode(routesOption || argument);
      }
    }
  }
});
extractJsxRoutes(ast);

walk(ast, (node, parent, ancestors) => {
  if (node.type === "VariableDeclarator" && node.id?.type === "Identifier" && node.init) {
    const name = node.id.name;
    const unwrappedInit = unwrap(node.init);
    if (unwrappedInit?.type === "NewExpression" && memberName(unwrappedInit.callee) === "Headers") {
      headerContainers.add(name);
      extractHeaderEntries(unwrappedInit.arguments?.[0], node, "headers-constructor");
    }
    const value = evaluate(node.init);
    if (value?.value) collectNamedStringEvidence(node, name, value.value);
    if (/(?:^|\.)headers(?:\.(?:common|get|post|put|patch|delete))?$/i.test(name)) {
      extractHeaderEntries(node.right, node, "client-interceptor");
    }
    if (/(?:base.?url|api.?url|api.?base|endpoint|gateway|request|service|client|http|auth|token|upload)/i.test(name)) {
      addCodeSlice(node, ancestors, "dependency-definition", name);
    }
  }
  if (node.type === "AssignmentExpression") {
    const name = memberName(node.left);
    const value = evaluate(node.right);
    if (value?.value) collectNamedStringEvidence(node, name, value.value);
    const headerMatch = name.match(/(?:^|\.)(?:defaults\.)?headers(?:\.(?:common|get|post|put|patch|delete))?\.([^.]*)$/i);
    if (headerMatch?.[1]) {
      pushHeaderEvidence(headerMatch[1], value?.value || "<dynamic>", node, "client-default", !value || value.dynamic);
    }
    if (/(?:base.?url|api.?url|api.?base|endpoint|gateway|defaults\.baseURL)/i.test(name)) {
      addCodeSlice(node, ancestors, "dependency-definition", name);
    }
  }
  if (node.type === "ObjectProperty") {
    const name = propertyName(node.key);
    const value = evaluate(node.value);
    if (value?.value) collectNamedStringEvidence(node, name, value.value);
    if (name.toLowerCase() === "headers") extractHeaderEntries(node.value, node, "declared-headers");
  }
  if (node.type === "ImportDeclaration") {
    const value = evaluate(node.source)?.value;
    if (value) imports.push(value);
    for (const specifier of node.specifiers || []) {
      if (/axios/i.test(value || "") && specifier.local?.name) clients.set(specifier.local.name, "");
    }
  }
  if (node.type === "CallExpression" || node.type === "OptionalCallExpression") {
    const callee = memberName(node.callee);
    if (/^(?:localStorage|sessionStorage)\.(?:getItem|setItem)$/.test(callee)) {
      for (const argument of node.arguments || []) {
        const value = evaluate(argument)?.value;
        if (value && (/^https?:\/\//i.test(value) || value.startsWith("/"))) {
          pushStringEvidence("storageReferences", value, node, callee, "medium");
        }
      }
    }
    if (callee === "require" || callee === "import") {
      const value = evaluate(node.arguments[0])?.value;
      if (value) imports.push(value);
    }
    if (callee.endsWith(".create") && /(?:axios|request|http|service|client)/i.test(callee)) {
      const owner = parent?.type === "VariableDeclarator" && parent.id?.type === "Identifier" ? parent.id.name : "";
      const base = evaluate(objectProperty(node.arguments[0], "baseURL"))?.value || "";
      if (owner) clients.set(owner, base);
      const defaults = extractHeaderEntries(objectProperty(node.arguments[0], "headers"), node, "client-default");
      if (owner && defaults.length) clientHeaders.set(owner, defaults);
      if (base) {
        baseUrls.push(base);
        pushStringEvidence(/^https?:\/\//i.test(base) ? "baseUrls" : "apiPrefixes", base, node, `${owner || callee}.baseURL`, "high");
      }
      addCodeSlice(node, ancestors, "http-client", `${callee} baseURL=${base}`);
    }
    if (callee.endsWith(".setRequestHeader") && node.arguments.length >= 2) {
      const name = evaluate(node.arguments[0]);
      const value = evaluate(node.arguments[1]);
      if (name?.value) pushHeaderEvidence(name.value, value?.value || "<dynamic>", node, "xhr-set-request-header", !value || value.dynamic);
      addCodeSlice(node, ancestors, "http-client", `${callee} ${name?.value || "dynamic-header"}`);
    }
    const parts = callee.split(".");
    const method = parts.at(-1)?.toLowerCase() || "";
    const owner = parts[0] || "";
    if (["set", "append"].includes(method) && (headerContainers.has(owner) || /headers?/i.test(owner)) && node.arguments.length >= 2) {
      const name = evaluate(node.arguments[0]);
      const value = evaluate(node.arguments[1]);
      if (name?.value) pushHeaderEvidence(name.value, value?.value || "<dynamic>", node, "headers-api", !value || value.dynamic);
      addCodeSlice(node, ancestors, "http-client", `${callee} ${name?.value || "dynamic-header"}`);
    }
    if (callee === "fetch" || callee.endsWith(".fetch")) {
      const config = node.arguments[1];
      addApi(node, node.arguments[0], evaluate(objectProperty(config, "method"))?.value || "GET", config);
      addCodeSlice(node, ancestors, "network-call", callee);
    } else if (method === "open" && node.arguments.length >= 2) {
      addApi(node, node.arguments[1], evaluate(node.arguments[0])?.value || "UNKNOWN", null, "babel-ast-xhr");
      addCodeSlice(node, ancestors, "network-call", callee);
    } else if (["get", "post", "put", "patch", "delete", "head", "options"].includes(method) && (clients.has(owner) || /^(?:axios|request|http|service|client|api|\$http|\$)/i.test(owner))) {
      addApi(node, node.arguments[0], method, node.arguments[1]);
      addCodeSlice(node, ancestors, "network-call", callee);
    } else if (["request", "ajax"].includes(method) || ["request", "ajax"].includes(callee)) {
      const config = node.arguments[0];
      const urlNode = objectProperty(config, "url") || node.arguments[0];
      // jQuery uses `type: "POST"` while fetch/axios normally use `method`.
      // Treat both spellings as the request method so a POST-only business API
      // is not downgraded to an UNKNOWN candidate and then discarded by the
      // safe GET verifier.
      const requestMethod = evaluate(objectProperty(config, "method") || objectProperty(config, "type"))?.value || "UNKNOWN";
      addApi(node, urlNode, requestMethod, config);
      addCodeSlice(node, ancestors, "network-call", callee);
    } else if (clients.has(callee) || /^(?:axios|request|service|client)$/i.test(callee)) {
      const config = node.arguments[0];
      addApi(node, objectProperty(config, "url"), evaluate(objectProperty(config, "method") || objectProperty(config, "type"))?.value || "UNKNOWN", config);
      addCodeSlice(node, ancestors, "network-call", callee);
    } else if (/(?:interceptors?|auth|token|session|login|upload|download|export|router)/i.test(callee)) {
      addCodeSlice(node, ancestors, "business-flow", callee);
    }
  }
  if (node.type === "ObjectProperty" && propertyName(node.key) === "path" && !structuredRouteObjects.has(parent)) {
    const value = evaluate(node.value);
    const object = parent?.type === "ObjectExpression" ? parent : null;
    const routeShape = object && ["component", "children", "redirect", "name", "element", "loader"].some((key) => objectProperty(object, key));
    if (value?.value && routeShape && value.value.length <= 1000) {
      const normalized = joinRoutePath("", value.value);
      addRouteRecord(normalized, value.value, "", node, value.dynamic);
    }
  }
});

let moduleCount = 0;
walk(ast, (node) => {
  if (node.type !== "CallExpression" || !memberName(node.callee).endsWith(".push")) return;
  const argument = node.arguments[0];
  if (argument?.type !== "ArrayExpression") return;
  const modules = argument.elements?.find((item) => item?.type === "ObjectExpression");
  if (modules) moduleCount = Math.max(moduleCount, modules.properties.length);
});

function dedupe(items, key) {
  const seen = new Set();
  return items.filter((item) => { const marker = key(item); if (seen.has(marker)) return false; seen.add(marker); return true; });
}

process.stdout.write(JSON.stringify({
  apis: dedupe(apis, (item) => `${item.method}|${item.path}|${item.clientBaseUrl}`),
  imports: dedupe(imports.filter(Boolean), String),
  routes: dedupe(routes, (item) => `${item.path}|${item.source}`),
  baseUrls: dedupe(baseUrls.filter(Boolean), String),
  stringEvidence: Object.fromEntries(Object.entries(stringEvidence).map(([kind, items]) => [kind, dedupe(items, (item) => `${item.value}|${item.source}|${item.label}`).slice(0, 80)])),
  headerEvidence: dedupe(headerEvidence, (item) => `${item.name.toLowerCase()}|${item.value}|${item.sourceKind}|${item.source}|${item.line}`).slice(0, 160),
  codeSlices: dedupe(codeSlices, (item) => item.id)
    .sort((left, right) => right.context.length - left.context.length)
    .slice(0, 16),
  moduleCount,
  parseErrors: parseErrors.slice(0, 10),
}));
