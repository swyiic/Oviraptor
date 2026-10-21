import { readFile } from 'node:fs/promises';
import { startStdioWorker } from './sdk/worker.mjs';
import { createFixtureAuthorityApplication } from './application.mjs';

const manifest = JSON.parse(await readFile(new URL('./manifest.json', import.meta.url), 'utf8'));
await startStdioWorker(createFixtureAuthorityApplication(manifest)).closed;
