# Assets application page

This entry builds the mature Assets-owned Vue views into a self-contained
opaque-origin page. The desktop and app use the same business components and
facades; this directory does not contain a second inventory implementation.

```sh
npm --prefix apps/assets run check:ui
npm --prefix apps/assets run build:ui
```

Vue, icons and CSS are bundled into `dist/page.html`. No external module fetch,
Tauri runtime, Scanning implementation, desktop global i18n or Host domain code
is included. The authoring build records the actual runtime source closure,
including CSS imports, in `dist/build-manifest.json` and fails on forbidden
runtime imports. Dependencies are declared by this app; an installer must never
execute build scripts supplied by an installed package.

The public SDK `page-client` provides the nonce/source-bound message channel.
`assets_context` must supply an explicit trusted Host-to-native project mapping.
The page does not assume their IDs match. Its finite operator action list keeps
the 84 reviewed old facade commands and adds context/profile metadata reads.
Calls preserve raw DTOs and omit optional object properties as desktop JSON
serialization does. Native scope and permission enforcement remain authoritative.
No arbitrary command, path, HTTP or Host-token proxy is exposed to the iframe.

`src/features/assets/uiPorts.ts` is the normal desktop implementation. This
build maps only that explicit port to app storage/dialog/action implementations;
there is no AST transformation of business behavior. The app uses in-memory
presentation preferences and real DOM dialogs, without `allow-same-origin`,
`allow-modals` or `allow-forms`. Confirmations that outlive project/revision changes
are refused. Closing the page releases UI state without cancelling app jobs.

The UI includes inventory/filtering/response groups, targets and collection,
same-run recovery/cancellation, task receipts/logs/events, rules, ownership and
Exposure views. A mounted view is not proof its native action has been connected.
The native Assets application process, all ownership/Exposure actions, controlled
export retrieval, durable preferences, controlled external opening and explicit
Assets-to-Scanning handoff require independent integration acceptance. Until
connected, errors stay visible rather than reporting fake successful operations.

The current browser evidence uses the real bundled Vue views and SDK in a real
Chromium opaque iframe, with an explicitly synthetic local parent/action bridge.
It proves UI behavior and sandbox constraints, not native DB/provider execution.
The original desktop remains available throughout migration.
