# org.oviraptor.hackerone

Minimal installable application package for HackerOne.

- **Read-only tools**: `programs_list`, `program_get` against an empty local cache by default.
- **No attack / exploit / payload surface** in this package.
- Desktop Tauri `src-tauri` HackerOne business kernel is intentionally untouched; shell catalog points at this host app id when installed.
- `paired_skills` references `docs/operator-guide.md` for install-time Skill preview/import.
