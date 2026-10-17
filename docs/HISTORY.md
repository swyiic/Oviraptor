# Docs history pointer

Historical dated audits (`NEST_*`, agent-first snapshots, asset-exposure deliveries,
general audits, task prompts, small integration JSON) and the old stubs
`docs/architecture.md`, `docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md`, and
`docs/progress.md` were removed from the working tree on **2026-10-17** (Asia/Shanghai).

**Recoverable via annotated tag** `archive/docs-historical-20261017` (commit before deletion).

```bash
git checkout archive/docs-historical-20261017 -- docs/archive \
  docs/architecture.md docs/OVIRAPTOR_AGENT_RUNTIME_ARCHITECTURE.md docs/progress.md
```

Living docs remain: root `ARCHITECTURE.md`, `docs/APP_DEVELOPER_GUIDE.md`,
`docs/OFFLOAD_NOTE_20261017.md`, and package/apps READMEs.
