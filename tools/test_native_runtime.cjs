#!/usr/bin/env node
"use strict";

// Local-only integration fixture; never loads a user's target or credentials.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");
const { spawn } = require("node:child_process");
const root = path.resolve(__dirname, "..");
const helper = path.join(root, "src-tauri/resources/workers/9_frontend_runtime_probe.cjs");
const fixture = fs.readFileSync(path.join(root, "src-tauri/resources/workers/tests/fixtures/runtime_probe.html"));
const requests = [];
const externalHits = [];
const externalServer = http.createServer((request, response) => {
  externalHits.push(request.url);
  response.writeHead(200, { "Content-Type": "image/png" });
  response.end("");
});
const server = http.createServer((request, response) => {
  requests.push({ method: request.method, url: request.url, headers: request.headers });
  if (request.url === "/redirect-outside") {
    response.writeHead(302, { Location: `http://127.0.0.1:${externalServer.address().port}/redirected-outside.png` });
    response.end();
  } else if (request.url.startsWith("/api/")) {
    response.writeHead(200, { "Content-Type": "application/json" });
    response.end(JSON.stringify({ id: 42, name: "local fixture", account: request.headers["x-private-fixture-key"] || "anonymous" }));
  } else {
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(`${fixture.toString()}<img src="http://127.0.0.1:${externalServer.address().port}/outside-scope.png"><img src="/redirect-outside">`);
  }
});

function run(payload) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [helper], { stdio: ["pipe", "pipe", "pipe"], detached: process.platform !== "win32" });
    let output = "", errors = "";
    const timer = setTimeout(() => {
      try { process.kill(process.platform === "win32" ? child.pid : -child.pid, "SIGKILL"); } catch {}
      reject(new Error("local CDP fixture exceeded 55 seconds"));
    }, 55000);
    child.stdout.on("data", chunk => { output += chunk; });
    child.stderr.on("data", chunk => { errors = (errors + chunk).slice(-2000); });
    child.on("error", error => { clearTimeout(timer); reject(error); });
    child.on("exit", code => {
      clearTimeout(timer);
      if (code !== 0) return reject(new Error(`CDP helper exit ${code}: ${errors}`));
      try { resolve(JSON.parse(output)); } catch (error) { reject(error); }
    });
    child.stdin.on("error", () => {});
    child.stdin.end(JSON.stringify(payload));
  });
}

(async () => {
  await new Promise(resolve => externalServer.listen(0, "127.0.0.1", resolve));
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const url = `http://127.0.0.1:${server.address().port}/`;
  try {
    const common = { url, timeoutMs: 7000, explorationTimeoutMs: 18000, settleMs: 300, maxActions: 12, maxStates: 8, maxDepth: 2 };
    const anonymous = await run(common);
    assert.equal(anonymous.available, true, JSON.stringify(anonymous.errors));
    assert.equal(anonymous.captureStatus, "complete");
    assert.equal(anonymous.authSessionValidation.applied, false);
    assert.ok(anonymous.requests.some(request => request.url.includes("/api/v1/") && request.status === 200), "click-triggered API response must be captured");
    assert.ok(!requests.some(request => request.method === "POST"), "fixture write action must not be sent");
    assert.equal(externalHits.length, 0, "browser must block cross-origin subresources before they reach a second local server");
    assert.ok(anonymous.blockedRequests.some(request => request.safetyReason === "outside_target_origin"), "blocked cross-origin request must be visible in the browser audit");
    assert.ok(requests.some(request => request.url === "/redirect-outside"), "redirect fixture must actually exercise the browser");
    const previous = requests.length;
    const comparison = await run({ ...common, comparisonOnly: true, authSession: { id: "fixture-b", scopeHosts: ["127.0.0.1"], headers: { "X-Private-Fixture-Key": "identity-b" } }, comparisonRequests: [{ method: "GET", url: `${url}api/v1/profile`, headers: { "X-Private-Fixture-Key": "identity-a", Authorization: "Bearer fixture-a", Accept: "application/json" } }] });
    const replay = comparison.comparisonReplays?.[0];
    assert.equal(replay?.outcome, "completed", JSON.stringify({ captureStatus: comparison.captureStatus, captureError: comparison.captureError, errors: comparison.errors, stopReason: comparison.stopReason, replays: comparison.comparisonReplays }));
    assert.equal(replay?.status, 200);
    assert.equal(replay?.responseBodyCaptured, true);
    assert.equal(JSON.parse(replay.responsePreview).account, "identity-b");
    const captured = requests.slice(previous).find(request => request.url === "/api/v1/profile");
    assert.equal(captured?.headers["x-private-fixture-key"], "identity-b");
    assert.equal(captured?.headers.authorization, undefined);
    console.log(JSON.stringify({ passed: true, anonymousCapture: anonymous.captureStatus, observedRequests: anonymous.requests.length, comparisonCapture: comparison.captureStatus, isolatedIdentity: true }));
  } finally {
    server.closeAllConnections(); externalServer.closeAllConnections();
    await Promise.all([new Promise(resolve => server.close(resolve)), new Promise(resolve => externalServer.close(resolve))]);
  }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
