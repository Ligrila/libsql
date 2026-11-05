//! Opt-in regression against a disposable sqld namespace, never production data.
#![cfg(feature = "replication")]

use libsql::params::IntoParams;
use libsql::{params, Builder, Connection};

async fn count(conn: &Connection, table: &str) -> i64 {
    let mut rows = conn
        .query(&format!("SELECT count(*) FROM {table}"), ())
        .await
        .unwrap();
    rows.next().await.unwrap().unwrap().get(0).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires SQLD_TEST_URL and SQLD_TEST_NAMESPACE on a disposable sqld"]
async fn replica_steps_should_rollback_and_catch_up_to_the_committed_write() {
    let url = std::env::var("SQLD_TEST_URL").expect("SQLD_TEST_URL");
    let namespace = std::env::var("SQLD_TEST_NAMESPACE").expect("SQLD_TEST_NAMESPACE");
    let token = std::env::var("SQLD_TEST_TOKEN").unwrap_or_default();
    let dir = tempfile::tempdir().unwrap();
    let db = Builder::new_remote_replica(dir.path().join("replica.db"), url, token)
        .namespace(namespace)
        .read_your_writes(false)
        .build()
        .await
        .unwrap();
    let conn = db.connect().unwrap();
    let table = format!("transactional_steps_{:016x}", rand::random::<u64>());
    conn.execute(
        &format!("CREATE TABLE {table} (id TEXT PRIMARY KEY, n INTEGER NOT NULL)"),
        (),
    )
    .await
    .unwrap();
    db.sync().await.unwrap();
    let insert = format!("INSERT INTO {table} VALUES (?1, ?2)");
    let failed = conn
        .execute_transactional_steps(vec![
            (insert.clone(), params!["a", 1].into_params().unwrap()),
            (insert.clone(), params!["a", 2].into_params().unwrap()),
        ])
        .await;
    assert!(failed.is_err());
    assert!(conn.is_autocommit());
    db.sync().await.unwrap();
    assert_eq!(count(&conn, &table).await, 0);

    let affected = conn
        .execute_transactional_steps(vec![
            (insert.clone(), params!["a", 1].into_params().unwrap()),
            (insert, params!["b", 2].into_params().unwrap()),
        ])
        .await
        .unwrap();
    assert_eq!(affected, vec![1, 1]);
    let written = db
        .max_write_replication_index()
        .expect("write replication index");
    let reached = db.catch_up_to(written).await.unwrap();
    assert!(reached.unwrap_or(0) >= written);
    assert_eq!(count(&conn, &table).await, 2);
    assert_eq!(db.catch_up_to(written).await.unwrap(), reached);
    assert!(conn.is_autocommit());
    conn.execute(&format!("DROP TABLE {table}"), ())
        .await
        .unwrap();
}
