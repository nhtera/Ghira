# ghi-store fixtures

`schema/vN.db`: one small SQLCipher database per shipped schema version (v1 ..
latest), encrypted with the TEST key ring in `tests/common/mod.rs`
(`fixture_ring`); never a real key or real data. `tests/schema_upgrade.rs`
opens each with the current code and upgrades it.

Regenerate (applies `MIGRATIONS[..N]` to an empty DB and seeds rows; random
nonces make the bytes differ on every run, so only do it when a version is added):

    cargo test -p ghi-store --test schema_upgrade -- --ignored regenerate_fixtures
