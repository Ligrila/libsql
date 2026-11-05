//! Parameterized, atomic transactions on the local backend.

use libsql::params::{IntoParams, Params};
use libsql::{params, Builder, Connection};

fn step(sql: &str, params: impl IntoParams) -> (String, Params) {
    (sql.to_string(), params.into_params().expect("params"))
}

async fn count(conn: &Connection, sql: &str) -> i64 {
    let mut rows = conn.query(sql, ()).await.expect("query");
    rows.next()
        .await
        .expect("row")
        .expect("one row")
        .get::<i64>(0)
        .expect("integer")
}

async fn local() -> Connection {
    let db = Builder::new_local(":memory:").build().await.expect("open");
    let conn = db.connect().expect("connect");
    conn.execute(
        "CREATE TABLE t (id TEXT PRIMARY KEY, n INTEGER NOT NULL)",
        (),
    )
    .await
    .expect("table");
    conn
}

#[tokio::test]
async fn local_steps_should_apply_every_statement_with_its_own_parameters() {
    let conn = local().await;
    let affected = conn
        .execute_transactional_steps(vec![
            step("INSERT INTO t VALUES (?1, ?2)", params!["a", 1]),
            step("INSERT INTO t VALUES (?1, ?2)", params!["b", 2]),
            step("UPDATE t SET n = n + ?1", params![10]),
        ])
        .await
        .expect("steps");
    assert_eq!(affected, vec![1, 1, 2]);
    assert_eq!(count(&conn, "SELECT sum(n) FROM t").await, 23);
    assert!(conn.is_autocommit());
}

#[tokio::test]
async fn local_steps_should_roll_everything_back_when_one_fails() {
    let conn = local().await;
    let result = conn
        .execute_transactional_steps(vec![
            step("INSERT INTO t VALUES (?1, ?2)", params!["a", 1]),
            step("INSERT INTO t VALUES (?1, ?2)", params!["a", 2]),
            step("INSERT INTO t VALUES (?1, ?2)", params!["c", 3]),
        ])
        .await;
    assert!(result.is_err(), "a duplicate key must fail the call");
    assert_eq!(count(&conn, "SELECT count(*) FROM t").await, 0);
    assert!(conn.is_autocommit());
}

#[tokio::test]
async fn steps_should_refuse_a_statement_that_controls_the_transaction() {
    let conn = local().await;
    for sql in [
        "BEGIN",
        "commit",
        "  ROLLBACK",
        "SAVEPOINT s",
        "RELEASE s",
        "END",
    ] {
        let result = conn
            .execute_transactional_steps(vec![(sql.to_string(), Params::None)])
            .await;
        assert!(result.is_err(), "`{sql}` must be refused");
    }
}

#[tokio::test]
async fn steps_should_refuse_to_run_inside_an_open_transaction() {
    let conn = local().await;
    conn.execute("BEGIN", ()).await.expect("begin");
    let result = conn
        .execute_transactional_steps(vec![step("INSERT INTO t VALUES (?1, ?2)", params!["a", 1])])
        .await;
    assert!(result.is_err());
    conn.execute("ROLLBACK", ()).await.expect("rollback");
}

#[tokio::test]
async fn empty_steps_should_be_a_no_op() {
    let conn = local().await;
    assert!(conn
        .execute_transactional_steps(Vec::new())
        .await
        .unwrap()
        .is_empty());
    assert!(conn.is_autocommit());
}

#[cfg(feature = "replication")]
#[tokio::test]
async fn catch_up_should_reject_a_database_without_a_replica() {
    let db = Builder::new_local(":memory:").build().await.unwrap();
    assert!(db.catch_up_to(1).await.is_err());
}

#[tokio::test]
async fn commented_transaction_control_should_not_allow_partial_commits() {
    for sql in [
        "/* explanation */ COMMIT",
        "-- explanation\nCOMMIT",
        "; ; COMMIT",
        "\u{feff} /* first */ -- second\n ; COMMIT",
        "/* explanation */ END TRANSACTION",
        "-- explanation\nROLLBACK",
        "/* explanation */ SAVEPOINT nested",
        "/* explanation */ RELEASE nested",
        "/* explanation */ BEGIN",
    ] {
        let conn = local().await;
        let result = conn
            .execute_transactional_steps(vec![
                step("INSERT INTO t VALUES (?1, ?2)", params!["a", 1]),
                (sql.to_string(), Params::None),
                step("INSERT INTO t VALUES (?1, ?2)", params!["a", 2]),
            ])
            .await;
        assert!(result.is_err(), "`{sql}` must be refused");
        assert_eq!(
            count(&conn, "SELECT count(*) FROM t").await,
            0,
            "`{sql}` committed a partial batch"
        );
        assert!(conn.is_autocommit(), "`{sql}` left an open transaction");
    }
}

#[tokio::test]
async fn comments_before_regular_writes_should_remain_supported() {
    let conn = local().await;
    let affected = conn
        .execute_transactional_steps(vec![
            step("/* write */ INSERT INTO t VALUES (?1, ?2)", params!["a", 1]),
            step("-- write\nINSERT INTO t VALUES (?1, ?2)", params!["b", 2]),
        ])
        .await
        .unwrap();
    assert_eq!(affected, vec![1, 1]);
    assert_eq!(count(&conn, "SELECT count(*) FROM t").await, 2);
}
