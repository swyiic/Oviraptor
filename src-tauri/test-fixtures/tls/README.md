Synthetic self-signed localhost certificate and PUBLIC TEST-ONLY private key. Never use this key for real TLS or credentials. Generated with OpenSSL for native HTTPS regression tests; SHA-256 oracle comes from openssl x509 -fingerprint -sha256.

`many-san.der` is a synthetic certificate signed with the same public test-only key. It has 140 example.test SAN names followed by outside.invalid, to verify that an external SAN beyond entry 128 cannot disappear from paid-query admission.

`cn-only.der` is a synthetic self-signed leaf whose only hostname is its Subject CN `fixture.example.test`; `ip-san.der` has that same CN and an IP-only SAN. They verify legacy CN fallback only when SAN is absent, and never let CN override an existing SAN extension. Neither certificate nor the public test-only key is suitable for deployment.

`cleanup-organization.der` is a synthetic self-signed, ten-year test leaf for the reversible seed-cleanup regression. Its Subject O is `Other Bank Limited`; SANs contain `owned.cleanup-fixture.test` and `*.tenant.cleanup-fixture.test`. It tests exact-host local certificate/ICP corroboration and wildcard protection. The generated private key was discarded; this certificate is never used for real identity verification or deployment.
