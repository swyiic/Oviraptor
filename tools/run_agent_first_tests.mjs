import {globSync,realpathSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {fileURLToPath} from 'node:url';
import {spawn} from 'node:child_process';

const root=fileURLToPath(new URL('../',import.meta.url));
const suites=globSync([
  'packages/app-sdk/test/*.test.mjs','packages/skills-library/test/*.test.mjs',
  'examples/echo-app/test/*.test.mjs','packages/runtime-acp/tests/*.test.mjs',
  'packages/agent-ui/test/*.test.mjs','packages/agent-host/test/*.test.mjs',
  'apps/*/test/*.test.mjs',
],{cwd:root}).sort();
if(!suites.length)throw Error('No Agent/application test suites discovered');
// Real app storage rejects alias directories. macOS often supplies /var as
// TMPDIR while its physical path is /private/var. Normalize the test parent,
// rather than weakening production storage checks or skipping positive cases.
const temporary=realpathSync(tmpdir());
const env={...process.env,...(process.platform==='win32'?{TEMP:temporary,TMP:temporary}:{TMPDIR:temporary})};
const child=spawn(process.execPath,['--test','--test-concurrency=1',...suites],{cwd:root,stdio:'inherit',env});
child.once('error',error=>{process.stderr.write(error.message+'\n');process.exitCode=1;});
child.once('exit',(code,signal)=>{process.exitCode=signal?1:code??1;});
