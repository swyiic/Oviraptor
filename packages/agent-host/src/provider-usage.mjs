const count = value => Number.isSafeInteger(value) && value >= 0 ? value : null;

/** Actual provider-reported tokens only. Missing counters remain unknown. */
export function normalizeProviderUsage(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const usage = {
    inputTokens: count(value.input_tokens ?? value.prompt_tokens),
    outputTokens: count(value.output_tokens ?? value.completion_tokens),
    totalTokens: count(value.total_tokens),
    cachedInputTokens: count(value.input_tokens_details?.cached_tokens ?? value.prompt_tokens_details?.cached_tokens ?? value.prompt_cache_hit_tokens),
    reasoningTokens: count(value.output_tokens_details?.reasoning_tokens ?? value.completion_tokens_details?.reasoning_tokens),
  };
  return Object.values(usage).some(v => v !== null) ? usage : null;
}
