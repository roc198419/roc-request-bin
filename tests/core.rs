use roc_request_bin::{server::Listener, store::*};

fn request(method: &str, url: &str) -> CapturedRequest {
    CapturedRequest {
        id: 0,
        time: "2026-11-13T00:00:00Z".into(),
        method: method.into(),
        url: url.into(),
        peer: "127.0.0.1:1234".into(),
        headers: vec![],
        body: vec![0, 255, 42],
    }
}
fn db() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let db = Store::open(&dir.path().join("data.db")).unwrap();
    (dir, db)
}

#[test]
fn prefix_is_unique_independently_of_methods_and_name() {
    let (_dir, db) = db();
    db.add_rule("A", " http://local/a ", &["GET".into()])
        .unwrap();
    let error = db
        .add_rule(
            "Different name",
            "https://other.example/a",
            &["POST".into()],
        )
        .unwrap_err();
    assert!(error.to_string().contains("URL 前缀已存在"));
    assert_eq!(db.rules().unwrap().len(), 1);
    assert_eq!(db.rules().unwrap()[0].prefix, "/a");
    db.add_rule("Child", "/a/b", &["GET".into()]).unwrap();
}
#[test]
fn matching_backfill_literal_prefix_and_cascade_are_consistent() {
    let (_dir, db) = db();
    db.insert(&request("POST", "http://local/a_%/child"))
        .unwrap();
    db.insert(&request("GET", "http://local/a_%/child"))
        .unwrap();
    db.insert(&request("POST", "http://local/abc/child"))
        .unwrap();
    let a = db.add_rule("Literal", "/a_%", &["POST".into()]).unwrap();
    let b = db
        .add_rule("Parent", "http://local/", &["POST".into(), "GET".into()])
        .unwrap();
    assert_eq!(db.page(Filter::Rule(a), 0).unwrap().0, 1);
    assert_eq!(db.page(Filter::Rule(b), 0).unwrap().0, 3);
    assert_eq!(db.page(Filter::Unmatched, 0).unwrap().0, 0);
    let id = db.insert(&request("POST", "http://local/a_%/new")).unwrap();
    assert_eq!(db.rules().unwrap()[0].count, 2);
    db.delete_requests(&[id]).unwrap();
    assert_eq!(db.rules().unwrap()[0].count, 1);
    db.delete_rules(&[b]).unwrap();
    assert_eq!(db.page(Filter::Unmatched, 0).unwrap().0, 2);
    assert_eq!(db.page(Filter::All, 0).unwrap().0, 3);
}
#[test]
fn restart_restores_settings_and_raw_bytes_and_pagination() {
    let (dir, db) = db();
    db.save_settings(&Settings {
        sidebar_ratio: 0.42,
        ip: "::1".into(),
        port: 9000,
    })
    .unwrap();
    for _ in 0..53 {
        db.insert(&request("POST", "http://local/")).unwrap();
    }
    drop(db);
    let db = Store::open(&dir.path().join("data.db")).unwrap();
    assert_eq!(db.settings().unwrap().sidebar_ratio, 0.42);
    let (count, page) = db.page(Filter::All, 1).unwrap();
    assert_eq!(count, 53);
    assert_eq!(page.len(), 3);
    assert_eq!(db.request(page[0].id).unwrap().body, vec![0, 255, 42]);
    assert!(page[0].id > page[1].id);
}

#[test]
fn migration_drops_description_normalizes_rules_and_preserves_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,description TEXT NOT NULL DEFAULT '',prefix TEXT NOT NULL,methods TEXT NOT NULL);
        INSERT INTO rules(name,prefix,methods) VALUES('old','http://local/api','[\"GET\"]'),('same path','https://other/api','[\"POST\"]');").unwrap();
    drop(conn);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.rules().unwrap().len(), 1);
    assert_eq!(db.rules().unwrap()[0].prefix, "/api");
    assert_eq!(db.rules().unwrap()[0].methods.len(), 2);
    db.insert(&request("GET", "https://third.example/api/orders"))
        .unwrap();
    assert_eq!(db.rules().unwrap()[0].count, 1);
    db.add_rule("new", "/new", &["POST".into()]).unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.rules().unwrap()[1].prefix, "/new");
    assert_eq!(db.page(Filter::All, 0).unwrap().0, 1);
    assert!(
        db.add_rule("duplicate", "http://local/new", &["GET".into()])
            .is_err()
    );
    let c = rusqlite::Connection::open(&path).unwrap();
    let columns = c
        .prepare("PRAGMA table_info(rules)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert!(!columns.iter().any(|c| c == "description"));
}

#[test]
fn matching_ignores_scheme_host_and_port_for_history_and_new_requests() {
    let (_dir, db) = db();
    db.insert(&request(
        "POST",
        "https://first.example:9443/api/orders?state=paid",
    ))
    .unwrap();
    let id = db
        .add_rule("orders", "/api/orders?state=", &["POST".into()])
        .unwrap();
    db.insert(&request(
        "POST",
        "http://second.example:8080/api/orders?state=pending",
    ))
    .unwrap();
    db.insert(&request(
        "GET",
        "http://second.example/api/orders?state=pending",
    ))
    .unwrap();
    db.insert(&request(
        "POST",
        "http://second.example/other/api/orders?state=pending",
    ))
    .unwrap();
    assert_eq!(db.page(Filter::Rule(id), 0).unwrap().0, 2);
    assert_eq!(db.page(Filter::Unmatched, 0).unwrap().0, 2);
    assert!(
        db.add_rule(
            "same",
            "https://third.example/api/orders?state=",
            &["GET".into()]
        )
        .is_err()
    );
}

#[test]
fn request_time_sorts_before_id_with_stable_ties() {
    let (_dir, db) = db();
    let mut r = request("GET", "http://local/");
    r.time = "2026-11-13T12:00:00.000Z".into();
    let newest = db.insert(&r).unwrap();
    r.time = "2026-11-13T10:00:00.000Z".into();
    let oldest = db.insert(&r).unwrap();
    r.time = "2026-11-13T11:00:00.000Z".into();
    let middle = db.insert(&r).unwrap();
    let tie = db.insert(&r).unwrap();
    let (_, rows) = db.page(Filter::All, 0).unwrap();
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![newest, tie, middle, oldest]
    );
}
#[tokio::test]
async fn real_http_captures_duplicate_headers_binary_body_and_bind_errors() {
    let (_dir, db) = db();
    let listener = Listener::start(db.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
    assert!(Listener::start(db.clone(), listener.address).is_err());
    let url = format!("http://{}/webhook?q=a%20b", listener.address);
    let response = reqwest::Client::new()
        .post(&url)
        .header("x-repeat", "one")
        .header("x-repeat", "two")
        .body(vec![0, 255, 42])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let (_, rows) = db.page(Filter::All, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].url, url);
    assert_eq!(rows[0].method, "POST");
    let captured = db.request(rows[0].id).unwrap();
    assert_eq!(captured.body, vec![0, 255, 42]);
    assert_eq!(
        captured
            .headers
            .iter()
            .filter(|h| h.name == "x-repeat")
            .count(),
        2
    );
}
#[tokio::test]
async fn oversized_body_is_rejected_without_partial_capture() {
    let (_dir, db) = db();
    let listener = Listener::start(db.clone(), "127.0.0.1:0".parse().unwrap()).unwrap();
    let response = reqwest::Client::new()
        .post(format!("http://{}/", listener.address))
        .body(vec![42; roc_request_bin::server::MAX_BODY + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(db.page(Filter::All, 0).unwrap().0, 0);
}
