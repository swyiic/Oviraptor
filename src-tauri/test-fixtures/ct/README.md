# Public CT parser regression fixtures

`public-leaf.pem` and `precertificate.pem` are synthetic public certificates
without private keys. They test independent scope and fingerprint admission.

The `crtsh-*.pem` files are unmodified public responses captured on 2026-10-23
from `https://crt.sh/?d=<id>` while diagnosing the installed application's queue.
IDs 29412545097, 29253617849, 21711555305 and 21688908177 are RFC6962 poison
precertificates. ID 29581274041 is a final leaf with embedded SCTs; the SCT
extension must not be confused with the critical poison extension.

These fixtures contain public certificate identities only, not project data,
credentials or private keys. Tests perform no network or paid API request.
