// Trigger a real imported bridge's stdin failure handler without account access.
import '../../src/mcp-bridge.mjs';
process.stdin.emit('error', new Error('synthetic-private-input-error'));
