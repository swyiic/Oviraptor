# Project seed layout regression

This local-only check renders the production `ProjectSeedsWorkspace.vue`, section navigation and shell styles. Its IPC replies contain synthetic data; it does not read the project database, call a model, or contact a provider. Browser requests outside its local Vite origin are blocked.

Use an existing Playwright installation and Chromium binary; this check never installs dependencies or changes network settings:

```sh
OVI_PLAYWRIGHT_MODULE=/absolute/path/to/node_modules/playwright \
OVI_BROWSER_EXECUTABLE=/absolute/path/to/chromium \
node tools/qa/project-seeds/browser-qa.mjs
```

`OVI_BROWSER_EXECUTABLE` is optional when Playwright's matching browser is already installed. The JSON result covers initial selection, document scrolling, long independently scrollable history, preserving the selected fingerprint, keyboard focus restoration, navigation wrapping after a sidebar width change without resizing the window, observer/listener disposal on leaving the page, and viewport sizes 1700×1000, 1180×650, 1000×720, 600×620 and 390×560. The responsive cases must keep detail controls below shell navigation and inside the viewport. Only seed list/detail reads are allowed.

The regular UI suite additionally exercises real SFC behavior with scripted native IPC:

```sh
node --test tools/test_project_seeds_ui.cjs
```

Neither check replaces the installed App and real-project verification.
