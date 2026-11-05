# Ligrila libSQL Rust client fork

This public fork adds two Rust client APIs to libSQL **0.9.30**. It preserves
upstream history at `0653c5788d77ef16a97c56ff3e9fdc11717a72d9`, the source commit
recorded in the published crate's `.cargo_vcs_info.json`. The upstream MIT license
and copyright notice remain in `LICENSE.md`.

## Scope

Only the Rust client is extended. No sqld server changes, application code,
credentials or production data are included. The client is an independent Cargo
workspace in `libsql/`; its dependencies use the same registry versions declared
by the published 0.9.30 crate, not the monorepo's path dependencies. The remaining
upstream tree is retained for provenance, not as a promise of server maintenance.

### Parameterized atomic writes

`Connection::execute_transactional_steps(Vec<(String, Params)>) -> Result<Vec<u64>>`
runs a sequence of writes with distinct bound parameters in one transaction,
returning the affected row count for each statement. On an embedded replica,
the client sends a single replication program containing BEGIN IMMEDIATE,
conditional statements, COMMIT and a conditional ROLLBACK. The local backend
uses an ordinary transaction. Unsupported connection types return an error.

Call it outside an existing transaction. Each step must be a single statement,
not transaction-control SQL. The guard handles leading comments, empty statement
separators and a UTF-8 BOM, so those cannot hide a premature COMMIT. Values belong
in parameters, not SQL literals.

```rust,no_run
use libsql::params::IntoParams;
use libsql::{params, Builder};

# async fn example() -> libsql::Result<()> {
let db = Builder::new_local(":memory:").build().await?;
let conn = db.connect()?;
conn.execute("CREATE TABLE items (id TEXT PRIMARY KEY, quantity INTEGER)", ()).await?;
let affected = conn.execute_transactional_steps(vec![
    ("INSERT INTO items VALUES (?1, ?2)".into(), params!["a", 1].into_params()?),
    ("INSERT INTO items VALUES (?1, ?2)".into(), params!["b", 2].into_params()?),
]).await?;
assert_eq!(affected, vec![1, 1]);
# Ok(())
# }
```

### Catch up to a known write index

`Database::catch_up_to(frame_no) -> Result<Option<FrameNo>>` asks an embedded
replica to pull frames until it reaches the requested index or stops making
progress. Unlike `sync`, it does not force an initial handshake just to discover
the primary's latest index. A handshake may still be required by the server.

After a write, obtain the target with `max_write_replication_index()`. The returned
committed index must be checked: it is **not** a guarantee that the requested
index was reached. This method requires the `replication` feature and an embedded
remote replica; other database modes return an error.

## Build and tests

Use a current stable Rust toolchain explicitly; the upstream monorepo's historical
toolchain pin is retained unchanged.

```sh
cargo +stable test --manifest-path libsql/Cargo.toml --test transactional_steps \
  --features replication,hrana,encryption
```

The opt-in replica regression requires an existing **disposable** sqld namespace.
It creates and drops a randomly named table. Never point it at production.
`SQLD_TEST_TOKEN` is optional for servers without authentication.

```sh
SQLD_TEST_URL=http://127.0.0.1:8080 SQLD_TEST_NAMESPACE=tests \
  cargo +stable test --manifest-path libsql/Cargo.toml \
  --test transactional_steps_replica --features replication,hrana,encryption \
  -- --ignored --nocapture
```

Consumers can override crates.io using `[patch.crates-io]` with
`git = "https://github.com/Ligrila/libsql.git"` and `rev` set to a full, tested
commit SHA. Do not rely on a moving branch in production.

## Maintenance

Keep changes scoped to these APIs and their tests. Rebase updates deliberately
against a published client version, review dependency changes, and run both local
and disposable replica regressions. Generic improvements can be proposed upstream;
this fork does not imply upstream endorsement.
