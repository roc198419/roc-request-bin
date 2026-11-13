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
    db.add_rule("A", "支付回调说明", " http://local/a ", &["GET".into()])
        .unwrap();
    let error = db
        .add_rule("Different name", "", "http://local/a", &["POST".into()])
        .unwrap_err();
    assert!(error.to_string().contains("URL 前缀已存在"));
    assert_eq!(db.rules().unwrap().len(), 1);
    assert_eq!(db.rules().unwrap()[0].description, "支付回调说明");
    db.add_rule("Child", "", "http://local/a/b", &["GET".into()])
        .unwrap();
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
    let a = db
        .add_rule("Literal", "", "http://local/a_%", &["POST".into()])
        .unwrap();
    let b = db
        .add_rule(
            "Parent",
            "",
            "http://local/",
            &["POST".into(), "GET".into()],
        )
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
fn description_migrates_existing_databases_without_losing_rules() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("data.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE rules(id INTEGER PRIMARY KEY AUTOINCREMENT,name TEXT NOT NULL,prefix TEXT NOT NULL,methods TEXT NOT NULL);
        INSERT INTO rules(name,prefix,methods) VALUES('old','http://local/','[\"GET\"]');").unwrap();
    drop(conn);
    let db = Store::open(&path).unwrap();
    assert_eq!(db.rules().unwrap()[0].description, "");
    db.add_rule(
        "new",
        "A persisted description",
        "http://local/new",
        &["POST".into()],
    )
    .unwrap();
    drop(db);
    let db = Store::open(&path).unwrap();
    assert_eq!(
        db.rules().unwrap()[1].description,
        "A persisted description"
    );
    assert!(
        db.add_rule("duplicate", "", "http://local/new", &["GET".into()])
            .is_err()
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
