Synthetic self-signed localhost certificate and PUBLIC TEST-ONLY private key. Never use this key for real TLS or credentials. Generated with OpenSSL for native HTTPS regression tests; SHA-256 oracle comes from openssl x509 -fingerprint -sha256.

`many-san.der` is a synthetic certificate signed with the same public test-only key. It has 140 example.test SAN names followed by outside.invalid, to verify that an external SAN beyond entry 128 cannot disappear from paid-query admission.
