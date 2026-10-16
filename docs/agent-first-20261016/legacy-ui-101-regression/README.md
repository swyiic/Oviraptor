# Legacy regression checkpoint

The initial complete 99-file run actually returned exit 1: 1679 tests, 1664 passed, three failed, 12 original macOS skips. Two failed because the API loader did not map the new explicit Assets desktop UI port. The third Chromium fixture failed because the cloud sandbox helper has incorrect ownership. Two Scanning inputs also changed during that run; it is not a coherent-source pass. Raw results and input guards are retained.

Only the exact Assets uiPorts path now maps to the same synthetic invoke. All five original API assertions and the entire native-runtime fixture remain unchanged. The actual two-file retry passed all six tests with 137 unchanged inputs. That retry explicitly used the existing loopback-only synthetic-fixture opt-in OVIRAPTOR_TEST_NO_SANDBOX=1 and /usr/bin/chromium; it establishes no product sandbox acceptance. The first collector error is retained; the corrected collector and actual successful retry are separate records.

The prior actual desktop Vue/TSC and Vite build passed with stable inputs. A complete fresh 99-file regression on the integrated source is still required.
