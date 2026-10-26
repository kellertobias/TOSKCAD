import { appendFileSync } from 'node:fs';
import { buildVersion, runRelease } from './run-semantic-release.mjs';

if (!process.env.GITHUB_OUTPUT) throw new Error('This command requires a GitHub Actions output file');
if (process.env.GITHUB_EVENT_NAME === 'push' && process.env.GITHUB_REF === 'refs/heads/main') {
  process.env.MULTICAD_RELEASE_OUTPUT = process.env.GITHUB_OUTPUT;
  await runRelease({ dryRun: true });
} else {
  appendFileSync(process.env.GITHUB_OUTPUT, `version=${buildVersion()}\nrelease=false\n`);
}
