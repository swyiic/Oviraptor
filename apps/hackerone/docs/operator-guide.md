# HackerOne operator guide

Reference-only guidance for the HackerOne application package.

- Use `programs_list` and `program_get` only against **locally cached** records inside this app.
- Official API sync and profile credentials stay in the desktop shell / Tauri configuration gates. This package does not receive those credentials.
- Skills and this guide grant **no execution**, no network access, and no vulnerability-exploit capability.
- Prefer adding scopes into a project through the existing desktop HackerOne board when available; this app page is a thin placeholder until deeper host bridging lands.
