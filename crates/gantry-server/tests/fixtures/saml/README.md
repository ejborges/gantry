# SAML test fixtures

Signed responses produced by `dev/saml-fixtures.sh` with `xmlsec1`, an implementation of XML
Signature that is independent of Gantry. The tests in `tests/saml.rs` verify them to show
that Gantry's canonicalization and signature checks agree with a second implementation.

`idp-key.pem` is a throwaway private key generated for these tests only. It protects
nothing. It is here so tests can sign variations of a response (fresh timestamps, attacks).
