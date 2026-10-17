// xAI Grok OAuth — reuses the public Grok-CLI client (same approach as OpenCode).
// Primary path: RFC 8628 device-code (open verification_uri, show user_code, poll token).
// Optional: PKCE + loopback helpers for environments where a local callback is reachable.
// Tokens are Bearer access_token against https://api.x.ai (OpenAI-compatible). No forged OAuth.

import { createHash, randomBytes } from 'node:crypto';
import { hostError } from './store.mjs';
import { proxyFetch, networkFetch } from './net-proxy.mjs';

export const XAI_OAUTH = Object.freeze({
  CLIENT_ID: 'b1a00492-073a-47ea-816f-4c329264a828',
  AUTHORIZE_URL: 'https://auth.x.ai/oauth2/authorize',
  TOKEN_URL: 'https://auth.x.ai/oauth2/token',
  DEVICE_URL: 'https://auth.x.ai/oauth2/device/code',
  SCOPE: 'openid profile email offline_access grok-cli:access api:access',
  REFERRER: 'oviraptor',
  DEVICE_CODE_GRANT: 'urn:ietf:params:oauth:grant-type:device_code',
  DEFAULT_INTERVAL_MS: 5_000,
  MIN_INTERVAL_MS: 1_000,
  SLOW_DOWN_INCREMENT_MS: 5_000,
  DEFAULT_EXPIRES_MS: 5 * 60 * 1000,
  POLL_SAFETY_MARGIN_MS: 3_000,
  ACCESS_REFRESH_SKEW_MS: 120_000,
  LOOPBACK_PORT: 56_121,
});

function authHeaders(userAgent = 'oviraptor/agent-host') {
  return {
    'Content-Type': 'application/x-www-form-urlencoded',
    Accept: 'application/json',
    'User-Agent': userAgent,
  };
}

function positiveSecondsToMs(value, defaultMs) {
  const seconds = Number(value);
  return Number.isFinite(seconds) && seconds > 0 ? seconds * 1000 : defaultMs;
}

function defaultSleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/** Base64url without padding (PKCE / state). */
export function base64Url(buffer) {
  return Buffer.from(buffer).toString('base64url');
}

export function createPkcePair() {
  const verifier = base64Url(randomBytes(32));
  const challenge = base64Url(createHash('sha256').update(verifier).digest());
  const state = base64Url(randomBytes(16));
  return { verifier, challenge, state };
}

/** Build authorize URL for optional PKCE loopback (xAI may reject non-whitelist redirect_uri). */
export function buildAuthorizeUrl({ redirectUri, challenge, state, referrer = XAI_OAUTH.REFERRER } = {}) {
  if (!redirectUri || !challenge || !state) throw hostError('host_oauth_pkce_invalid');
  const url = new URL(XAI_OAUTH.AUTHORIZE_URL);
  url.searchParams.set('response_type', 'code');
  url.searchParams.set('client_id', XAI_OAUTH.CLIENT_ID);
  url.searchParams.set('redirect_uri', redirectUri);
  url.searchParams.set('scope', XAI_OAUTH.SCOPE);
  url.searchParams.set('code_challenge', challenge);
  url.searchParams.set('code_challenge_method', 'S256');
  url.searchParams.set('state', state);
  url.searchParams.set('referrer', referrer);
  // OpenCode notes: non-whitelist clients need plan=generic to avoid loopback rejection.
  url.searchParams.set('plan', 'generic');
  return url.toString();
}

export async function requestDeviceCode({
  fetch: fetchImpl = proxyFetch,
  deviceUrl = XAI_OAUTH.DEVICE_URL,
  referrer = XAI_OAUTH.REFERRER,
  userAgent,
} = {}) {
  const response = await networkFetch(fetchImpl, deviceUrl, {
    method: 'POST',
    headers: authHeaders(userAgent),
    body: new URLSearchParams({
      client_id: XAI_OAUTH.CLIENT_ID,
      scope: XAI_OAUTH.SCOPE,
      referrer,
    }).toString(),
  });
  if (!response.ok) {
    const detail = await response.text().catch(() => '');
    throw hostError('host_oauth_device_start_failed', 502, { httpStatus: response.status, detail: detail.slice(0, 240) });
  }
  const json = await response.json();
  if (!json?.device_code || !json?.user_code || !json?.verification_uri) {
    throw hostError('host_oauth_device_response_invalid', 502);
  }
  return {
    device_code: String(json.device_code),
    user_code: String(json.user_code),
    verification_uri: String(json.verification_uri),
    verification_uri_complete: json.verification_uri_complete ? String(json.verification_uri_complete) : undefined,
    expires_in: Number(json.expires_in) || undefined,
    interval: Number(json.interval) || undefined,
  };
}

/**
 * One token-endpoint poll. Returns:
 *   { status: 'authorized', tokens }
 *   { status: 'pending', slowDown?: boolean }
 *   { status: 'denied' | 'expired' | 'error', error?, detail? }
 */
export async function pollDeviceCodeOnce(deviceCode, {
  fetch: fetchImpl = proxyFetch,
  tokenUrl = XAI_OAUTH.TOKEN_URL,
  userAgent,
} = {}) {
  if (!deviceCode) throw hostError('host_oauth_device_code_missing');
  const response = await networkFetch(fetchImpl, tokenUrl, {
    method: 'POST',
    headers: authHeaders(userAgent),
    body: new URLSearchParams({
      grant_type: XAI_OAUTH.DEVICE_CODE_GRANT,
      client_id: XAI_OAUTH.CLIENT_ID,
      device_code: deviceCode,
    }).toString(),
  });
  if (response.ok) {
    const tokens = await response.json();
    if (!tokens?.access_token) throw hostError('host_oauth_token_invalid', 502);
    return {
      status: 'authorized',
      tokens: {
        access_token: String(tokens.access_token),
        refresh_token: tokens.refresh_token ? String(tokens.refresh_token) : undefined,
        expires_in: Number(tokens.expires_in) || 3600,
        token_type: tokens.token_type,
        scope: tokens.scope,
        id_token: tokens.id_token,
      },
    };
  }
  const body = await response.json().catch(() => ({}));
  const error = String(body?.error || '');
  if (error === 'authorization_pending') return { status: 'pending' };
  if (error === 'slow_down') return { status: 'pending', slowDown: true };
  if (error === 'access_denied' || error === 'authorization_denied') return { status: 'denied', error };
  if (error === 'expired_token') return { status: 'expired', error };
  return {
    status: 'error',
    error: error || `http_${response.status}`,
    detail: String(body?.error_description || '').slice(0, 240),
  };
}

/** Poll until authorized / denied / expired / timeout. Injectable sleep/now for tests. */
export async function pollDeviceCodeUntilDone(device, {
  fetch: fetchImpl = proxyFetch,
  tokenUrl = XAI_OAUTH.TOKEN_URL,
  sleep = defaultSleep,
  now = () => Date.now(),
  userAgent,
  signal,
} = {}) {
  const expiresInMs = positiveSecondsToMs(device.expires_in, XAI_OAUTH.DEFAULT_EXPIRES_MS);
  const deadline = now() + expiresInMs;
  let intervalMs = Math.max(
    positiveSecondsToMs(device.interval, XAI_OAUTH.DEFAULT_INTERVAL_MS),
    XAI_OAUTH.MIN_INTERVAL_MS,
  );
  while (now() < deadline) {
    if (signal?.aborted) throw hostError('host_oauth_cancelled', 499);
    const result = await pollDeviceCodeOnce(device.device_code, { fetch: fetchImpl, tokenUrl, userAgent });
    if (result.status === 'authorized') return result.tokens;
    if (result.status === 'denied') throw hostError('host_oauth_denied', 403);
    if (result.status === 'expired') throw hostError('host_oauth_expired', 410);
    if (result.status === 'error') throw hostError('host_oauth_token_failed', 502, { error: result.error, detail: result.detail });
    if (result.slowDown) intervalMs += XAI_OAUTH.SLOW_DOWN_INCREMENT_MS;
    const remaining = Math.max(0, deadline - now());
    await sleep(Math.min(intervalMs + XAI_OAUTH.POLL_SAFETY_MARGIN_MS, remaining || intervalMs));
  }
  throw hostError('host_oauth_timeout', 408);
}

export async function refreshAccessToken(refreshToken, {
  fetch: fetchImpl = proxyFetch,
  tokenUrl = XAI_OAUTH.TOKEN_URL,
  userAgent,
} = {}) {
  if (!refreshToken) throw hostError('host_oauth_refresh_missing');
  const response = await networkFetch(fetchImpl, tokenUrl, {
    method: 'POST',
    headers: authHeaders(userAgent),
    body: new URLSearchParams({
      grant_type: 'refresh_token',
      refresh_token: refreshToken,
      client_id: XAI_OAUTH.CLIENT_ID,
    }).toString(),
  });
  if (!response.ok) {
    const detail = await response.text().catch(() => '');
    throw hostError('host_oauth_refresh_failed', 502, { httpStatus: response.status, detail: detail.slice(0, 240) });
  }
  const tokens = await response.json();
  if (!tokens?.access_token) throw hostError('host_oauth_token_invalid', 502);
  return {
    access_token: String(tokens.access_token),
    refresh_token: tokens.refresh_token ? String(tokens.refresh_token) : refreshToken,
    expires_in: Number(tokens.expires_in) || 3600,
  };
}

/** JWT exp check without signature verify — only for proactive refresh timing. */
export function accessTokenIsExpiring(token, skewMs = XAI_OAUTH.ACCESS_REFRESH_SKEW_MS, now = Date.now()) {
  if (!token || typeof token !== 'string') return false;
  const parts = token.split('.');
  if (parts.length < 2) return false;
  try {
    let payload = parts[1].replace(/-/g, '+').replace(/_/g, '/');
    while (payload.length % 4 !== 0) payload += '=';
    const claims = JSON.parse(Buffer.from(payload, 'base64').toString('utf8'));
    if (typeof claims?.exp !== 'number') return false;
    return claims.exp * 1000 <= now + Math.max(0, skewMs);
  } catch {
    return false;
  }
}

export function normalizeOAuthTokens(tokens, now = Date.now()) {
  if (!tokens?.access_token) throw hostError('host_oauth_token_invalid');
  return {
    accessToken: String(tokens.access_token),
    refreshToken: tokens.refresh_token ? String(tokens.refresh_token) : undefined,
    expiresAt: now + (Number(tokens.expires_in) || 3600) * 1000,
    tokenType: tokens.token_type ? String(tokens.token_type) : 'Bearer',
    scope: tokens.scope ? String(tokens.scope) : undefined,
  };
}

/** Public shape for pending device login (never includes device_code). */
export function publicDevicePending(pending) {
  if (!pending) return null;
  return {
    status: pending.status,
    userCode: pending.user_code,
    verificationUri: pending.verification_uri,
    verificationUriComplete: pending.verification_uri_complete,
    expiresAt: pending.expires_at,
    error: pending.error,
  };
}
