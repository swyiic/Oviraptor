export const PROTOCOL_VERSION = 1;
export const API_VERSION = '1.0.0';
export const MAX_MESSAGE_BYTES = 128 * 1024;
export const MAX_TOOL_RESULT_BYTES = 64 * 1024;
export const RPC_METHODS = Object.freeze(['describe', 'lifecycle/start', 'lifecycle/stop', 'tool/call']);

export class AppSdkError extends Error {
  constructor(code, message, { rpcCode = -32602, cause } = {}) {
    super(message, { cause });
    this.name = 'AppSdkError';
    this.code = code;
    this.rpcCode = rpcCode;
  }
}
export const plainObject = value => value !== null && typeof value === 'object'
  && (Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null);
export function fail(code, message, options) { throw new AppSdkError(code, message, options); }
export function keys(value, allowed, required, where, code = 'app_protocol_invalid') {
  if (!plainObject(value)) fail(code, `${where} must be an object`);
  for (const key of Object.keys(value)) if (!allowed.includes(key)) fail(code, `${where} has an unknown field`);
  for (const key of required) if (!Object.hasOwn(value, key)) fail(code, `${where}.${key} is required`);
}
export function text(value, maximum, where, { minimum = 1, pattern, code = 'app_protocol_invalid' } = {}) {
  if (typeof value !== 'string' || value.length < minimum || value.length > maximum
    || /[\u0000-\u001f\u007f]/u.test(value) || (pattern && !pattern.test(value))) {
    fail(code, `${where} is invalid`);
  }
  return value;
}
export function jsonBytes(value, maximum, code = 'app_protocol_invalid') {
  let encoded;
  try {
    encoded = JSON.stringify(value, (_key, item) => {
      if (typeof item === 'bigint' || typeof item === 'function' || typeof item === 'undefined'
        || typeof item === 'symbol' || (typeof item === 'number' && !Number.isFinite(item))) {
        fail(code, 'Value must contain only finite JSON data');
      }
      return item;
    });
  } catch (error) {
    if (error instanceof AppSdkError) throw error;
    fail(code, 'Value must contain finite, acyclic JSON data');
  }
  if (typeof encoded !== 'string' || Buffer.byteLength(encoded) > maximum) fail(code, 'JSON exceeds its byte budget');
  return encoded;
}
export function validateContext(value) {
  keys(value, ['hostBoundProjectId', 'grantRevision', 'operationId'], ['hostBoundProjectId', 'grantRevision', 'operationId'], 'context');
  if (!Number.isSafeInteger(value.hostBoundProjectId) || value.hostBoundProjectId < 1) fail('app_context_invalid', 'context.hostBoundProjectId must be a positive safe integer');
  if (!Number.isSafeInteger(value.grantRevision) || value.grantRevision < 0) fail('app_context_invalid', 'context.grantRevision must be a nonnegative safe integer');
  text(value.operationId, 128, 'context.operationId', { code: 'app_context_invalid' });
  return Object.freeze({ ...value });
}
export function rpcError(id, error) {
  const known = error instanceof AppSdkError;
  return { jsonrpc: '2.0', id, error: {
    code: known ? error.rpcCode : -32603,
    message: known ? error.message : 'App operation failed',
    data: { code: known ? error.code : 'app_internal_error' },
  } };
}
