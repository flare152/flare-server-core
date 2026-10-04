# API Key material contract

`flare_core_infra::auth::api_key` implements business-neutral API Key material;
it does not grant any IAM role, app binding, scope or runtime permission.

The only accepted wire shape is `flr_{live|test}_{key_id}_{secret}`:

- `key_id`: exactly 12 uppercase RFC4648 base32 characters (`A-Z`, `2-7`).
- `secret`: exactly 32 CSPRNG bytes, canonical unpadded base64url (43 characters).
- Total length is exactly65 ASCII bytes. Secret underscores are not field separators.

The digest is SHA-256 of the32 decoded pepper bytes followed by the32 decoded
secret bytes, not their textual encodings. The dedicated verifier pepper has no
default and must be independently randomly generated; never reuse the Control,
Cell context, or end-user JWT key. Control and authorized Social/Core verifiers
need the same pepper; Nginx and the Management-only Gateway do not. Only its
SHA-256 configuration fingerprint may accompany projected credential metadata.
A fingerprint mismatch must fail closed. P0 must back up this installation secret;
changing it requires an explicit credential migration, not a silent restart.

The verifier binds the digest to the expected public key ID and live/test kind,
using `subtle` for digest comparison. Callers additionally enforce the persisted
app/environment binding, enabled service account, revoked/expired state, exact
endpoint scope and trusted source-IP allowlist. It must not take app identity or
roles from caller-supplied headers or request bodies.

`ApiKeyToken` and `ApiKeyPepper` do not implement Serialize/Clone/Display. Debug
and errors omit secrets. Parsed and generated secret buffers are zeroized on
drop. `into_issuance_token` consumes the material and returns a zeroizing buffer;
the issuer must persist only the digest/metadata and reveal plaintext only in
the successful creation response. No read/list API may reconstruct a secret.
Database uniqueness handles public-ID collisions by generating a new material
value; an entropy-source failure is an error, never a deterministic fallback.

Regression commands (the shell wrapper may inject `RUST_LOG=warn`, which would
filter out the INFO span in the independent OTLP test):

```sh
rtk proxy env -u RUST_LOG cargo test --offline --workspace
rtk proxy cargo clippy --offline -p flare-core-infra --all-targets -- -D warnings
```

## Local policy primitive

`auth::api_key_policy::ApiKeyGrant` is an immutable, validated local grant.
`authorize` checks digest/kind/selector, pepper fingerprint, enabled account,
revocation, exact millisecond expiry (no refresh grace), expected application,
exact endpoint scope and trusted peer-IP CIDRs. Each cache hit must call it
again; a cached grant cannot extend expiry. Some(empty)/Some(wildcard) expected
application never means a default. None derives identity only from the grant.
Success returns only key/app/service-account identity, not material or roles.

Scopes are opaque exact strings here; Control owns the allowed Platform scope
vocabulary. No wildcard hierarchy, Management role conversion, JWT fallback,
Control fetch or implicit forwarded-IP trust is provided. CIDRs must have
canonical network addresses and bounded counts; mixed IPv4/IPv6 mapped-address
normalization is the caller's explicitly trusted network policy, not an auth
fallback. Grant Debug redacts digest/fingerprint; errors omit token/policy input.

This module alone is not CRED-02/MIG-05 acceptance. Receivers still need signed,
versioned, persisted projections; gateways need bounded local lookup/cache
(TTL1s, version invalidation, fail closed when refresh is unavailable), tenant
availability gates, and actual endpoint authorization. A SQL fixture or unit
test is not a containerized business API/revocation/outage test.

## Hash-only snapshot input

`auth::credential_snapshot` adds the transport-neutral storage record and
validated `CredentialSnapshot`: canonical bounded identifiers, positive BIGINT
versions, at most1024 unique keys,1MiB total JSON, strict mode/material/scopes/
expiry/CIDRs. Empty key sets are valid revocation snapshots; consumers must
persist their watermarks atomically with replacement. `CredentialKeyRecord`
is explicitly serializable hash-only input, never a token/pepper; validate it
again before authorization. Unknown/missing JSON fields are rejected, Debug
redacts material, and validated snapshots expose no mutable records.

Core and Social own their separate PG adapters and signed-worker RPCs; this
shared module has no Proto/SQLx dependency, lifecycle authority or tenant
availability fallback. Its four test groups include storage-to-policy roundtrip
and disabled/revoked rejection. Gateway cache/loaders and Platform endpoints
remain separate required work.

## Bounded local projection verification

`auth::credential_verifier` supplies `LocalCredentialReader` and a fixed-policy
`CachedApiKeyVerifier`. The business-neutral port accepts only hash-only records
atomically read with app binding and positive version on the local primary.
Core/Social own the adapters; neither this module nor the adapters call Control.
Do not point this reader at a lagging replica. Corrupt/mismatched rows fail closed.

Only selectors/grants are cached, never raw tokens, peppers or accepted identities.
Each request rechecks secret/mode/pepper fingerprint, exact app/scope, trusted IP
and UTC expiry after awaited IO. Positive TTL1s, negative TTL100ms,4096 entries,
64 fixed single-flight shards,16 concurrent cold reads/waiters and250ms timeouts
bound memory, IO and waiting. TTL begins before IO. An expired entry is never
used when refresh fails; configuration mismatch/storage failure gives unavailable,
not a default tenant or End User fallback. No raw record escapes verification.

Eight deterministic/paused-clock groups cover expiry on hits, revoke/disable,
unknown/malformed/flooded IDs, simultaneous misses, read saturation/timeout,
capacity eviction and independent request-policy checks. Core/Social additionally
run real signed gRPC→PG→cached auth→empty revoke/restart tests. The1s bound starts
at **local projection commit**, not the Control mutation; upstream outbox lag and
reconciliation must still be measured. Tenant/feature/resource gates must run
independently on every business request, and Platform must have separate pools/
bulkheads. Gateway composition and actual Platform endpoints are still unfinished.
