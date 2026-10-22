import { scrubSecrets } from './quality-prompt.mjs';

// Save the same bounded, scrubbed failure that is shown to the model. Never
// forward arbitrary Error properties, request headers or OS exceptions.
export function toolFailureReceipt(error, operationId) {
  const candidate = typeof error?.code === 'string' ? error.code.split(':')[0] : '';
  const code = /^(?:host_|app_|skill_package_)[a-z0-9_]+$/.test(candidate) ? candidate : 'host_tool_failed';
  const details = error?.details ?? {};
  const reason = typeof details.reason === 'string' ? scrubSecrets(details.reason).slice(0, 1000) : '';
  const phase = ['not_started', 'unknown', 'accepted'].includes(details.phase) ? details.phase : undefined;
  return { error: code, ...(reason ? { reason } : {}), ...(phase ? { phase } : {}),
    ...(typeof details.outcomeUnknown === 'boolean' ? { outcomeUnknown: details.outcomeUnknown } : {}),
    ...(typeof operationId === 'string' && operationId.length <= 128 ? { operationId } : {}),
    retryable: error?.retryable === true };
}
