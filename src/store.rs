use anyhow::{Result, bail};
use base64::{Engine, prelude::BASE64_STANDARD};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "CONNECT", "TRACE",
];
pub const PAGE_SIZE: usize = 50;

pub fn display_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|time| {
            time.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
                .format("%Y-%m-%d %H:%M:%S%.3f")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_owned())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub sidebar_ratio: f32,
    pub ip: String,
    pub port: u16,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sidebar_ratio: 0.3,
            ip: "127.0.0.1".into(),
            port: 8787,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Rule {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub methods: Vec<String>,
    pub count: usize,
    pub response: ResponseConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponseConfig {
    pub status_code: u16,
    pub content_type: String,
    pub response_body: String,
}
impl Default for ResponseConfig {
    fn default() -> Self {
        Self {
            status_code: 200,
            content_type: "text/plain; charset=utf-8".into(),
            response_body: "Captured by Roc Request Bin\n".into(),
        }
    }
}
impl ResponseConfig {
    fn validate(&self) -> Result<()> {
        if !(200..=599).contains(&self.status_code) {
            bail!("Status code 必须为 200–599");
        }
        if self.content_type.parse::<mime::Mime>().is_err()
            || axum::http::HeaderValue::from_str(&self.content_type).is_err()
        {
            bail!("请输入有效的 Content-Type，例如 application/json");
        }
        if matches!(self.status_code, 204 | 205 | 304) && !self.response_body.is_empty() {
            bail!("204、205、304 响应不允许包含 Body，请清空 Response body");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Header {
    pub name: String,
    pub value_base64: String,
}
impl Header {
    pub fn new(name: String, value: &[u8]) -> Self {
        Self {
            name,
            value_base64: BASE64_STANDARD.encode(value),
        }
    }
    pub fn display_value(&self) -> String {
        BASE64_STANDARD
            .decode(&self.value_base64)
            .map(|v| String::from_utf8_lossy(&v).into_owned())
            .unwrap_or_default()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedRequest {
    pub id: i64,
    pub time: String,
    pub method: String,
    pub url: String,
    pub peer: String,
    pub headers: Vec<Header>,
    pub body: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct RequestSummary {
    pub id: i64,
    pub time: String,
    pub method: String,
    pub url: String,
    pub header_count: usize,
    pub body_size: usize,
}
impl CapturedRequest {
    pub fn body_text(&self) -> String {
        match std::str::from_utf8(&self.body) {
            Ok(s) => s.to_owned(),
            Err(_) => format!("[Binary · Base64]\n{}", BASE64_STANDARD.encode(&self.body)),
        }
    }
    pub fn export(&self) -> String {
        format!(
            "{} {}\nTime: {}\nPeer: {}\n{}\n\n{}",
            self.method,
            self.url,
            format_args!("{} (UTC+8)", display_time(&self.time)),
            self.peer,
            self.headers
                .iter()
                .map(|h| format!("{}: {}", h.name, h.display_value()))
                .collect::<Vec<_>>()
                .join("\n"),
            self.body_text()
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Unmatched,
    Rule(i64),
}

/// Rules and requests share exactly the same origin-independent representation.
pub fn match_target(url: &str) -> Result<String> {
    let value = url.trim();
    if value.is_empty() || value.contains('#') {
        bail!("请输入有效的请求路径前缀，例如 /api/orders");
    }
    let uri: axum::http::Uri = value
        .parse()
        .map_err(|_| anyhow::anyhow!("无效的 URL 或路径前缀"))?;
    if !value.starts_with('/') && !matches!(uri.scheme_str(), Some("http" | "https")) {
        bail!("路径前缀必须以 / 开头，或填写完整 HTTP URL");
    }
    Ok(uri
        .path_and_query()
        .map(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .unwrap_or("/")
        .to_owned())
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version >= 1 {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let columns = tx
        .prepare("PRAGMA table_info(rules)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if columns.iter().any(|v| v == "description") {
        tx.execute_batch("ALTER TABLE rules DROP COLUMN description;")?;
    }
    tx.execute_batch("ALTER TABLE requests ADD COLUMN target TEXT NOT NULL DEFAULT '';")?;
    let requests = tx
        .prepare("SELECT id,url FROM requests")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, url) in requests {
        tx.execute(
            "UPDATE requests SET target=?1 WHERE id=?2",
            params![match_target(&url)?, id],
        )?;
    }
    let rules = tx
        .prepare("SELECT id,prefix,methods FROM rules ORDER BY id")?
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut canonical = std::collections::BTreeMap::<String, (i64, Vec<String>)>::new();
    for (id, prefix, methods) in rules {
        let target = match_target(&prefix)?;
        let methods: Vec<String> = serde_json::from_str(&methods)?;
        if let Some((_, existing)) = canonical.get_mut(&target) {
            for method in methods {
                if !existing.contains(&method) {
                    existing.push(method);
                }
            }
            tx.execute("DELETE FROM rules WHERE id=?1", [id])?;
        } else {
            canonical.insert(target, (id, methods));
        }
    }
    for (prefix, (id, methods)) in canonical {
        tx.execute(
            "UPDATE rules SET prefix=?1,methods=?2 WHERE id=?3",
            params![prefix, serde_json::to_string(&methods)?, id],
        )?;
    }
    // Rebuild history because dropping the origin can broaden a rule's matches.
    tx.execute_batch("DELETE FROM matches;
        INSERT INTO matches SELECT q.id,r.id FROM requests q JOIN rules r ON substr(q.target,1,length(r.prefix))=r.prefix
        WHERE EXISTS(SELECT 1 FROM json_each(r.methods) WHERE value=q.method);
        PRAGMA user_version=1;")?;
    tx.commit()?;
    Ok(())
}

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
    revision: Arc<AtomicU64>,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS settings (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS rules (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, prefix TEXT NOT NULL, methods TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS requests (id INTEGER PRIMARY KEY AUTOINCREMENT, time TEXT NOT NULL, method TEXT NOT NULL, url TEXT NOT NULL, peer TEXT NOT NULL, headers TEXT NOT NULL, body BLOB NOT NULL);
            CREATE TABLE IF NOT EXISTS matches (request_id INTEGER REFERENCES requests(id) ON DELETE CASCADE, rule_id INTEGER REFERENCES rules(id) ON DELETE CASCADE, PRIMARY KEY(request_id,rule_id));
            CREATE INDEX IF NOT EXISTS matches_rule ON matches(rule_id,request_id);
            CREATE INDEX IF NOT EXISTS requests_time_desc ON requests(time DESC,id DESC);
            CREATE UNIQUE INDEX IF NOT EXISTS rules_prefix_unique ON rules(prefix);")?;
        migrate(&mut conn)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            let tx = conn.transaction()?;
            tx.execute_batch("ALTER TABLE rules ADD COLUMN status_code INTEGER NOT NULL DEFAULT 200;
                ALTER TABLE rules ADD COLUMN content_type TEXT NOT NULL DEFAULT 'text/plain; charset=utf-8';
                ALTER TABLE rules ADD COLUMN response_body TEXT NOT NULL DEFAULT 'Captured by Roc Request Bin
';
                PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            revision: Arc::new(AtomicU64::new(0)),
        })
    }
    pub fn user() -> Result<Self> {
        Self::open(
            &dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("Cannot locate user directory"))?
                .join(".roc-request-bin/data.db"),
        )
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
    fn changed(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
    pub fn settings(&self) -> Result<Settings> {
        let c = self.conn.lock().unwrap();
        let value = c.query_row("SELECT value FROM settings WHERE id=1", [], |r| {
            r.get::<_, String>(0)
        });
        match value {
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        if !(0.15..=0.65).contains(&s.sidebar_ratio)
            || s.port == 0
            || s.ip.parse::<std::net::IpAddr>().is_err()
        {
            bail!("Invalid IP, port or sidebar ratio");
        }
        self.conn.lock().unwrap().execute(
            "INSERT INTO settings VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }
    pub fn rules(&self) -> Result<Vec<Rule>> {
        let c = self.conn.lock().unwrap();
        let mut q = c.prepare("SELECT r.id,r.name,r.prefix,r.methods,(SELECT count(*) FROM matches m WHERE m.rule_id=r.id),r.status_code,r.content_type,r.response_body FROM rules r ORDER BY r.id")?;
        Ok(q.query_map([], |r| {
            let m: String = r.get(3)?;
            Ok(Rule {
                id: r.get(0)?,
                name: r.get(1)?,
                prefix: r.get(2)?,
                methods: serde_json::from_str(&m).unwrap_or_default(),
                count: r.get::<_, i64>(4)? as usize,
                response: ResponseConfig {
                    status_code: r.get(5)?,
                    content_type: r.get(6)?,
                    response_body: r.get(7)?,
                },
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn add_rule(&self, name: &str, prefix: &str, methods: &[String]) -> Result<i64> {
        self.save_rule(None, name, prefix, methods, &ResponseConfig::default())
    }
    pub fn save_rule(
        &self,
        id: Option<i64>,
        name: &str,
        prefix: &str,
        methods: &[String],
        response: &ResponseConfig,
    ) -> Result<i64> {
        if name.trim().is_empty()
            || prefix.trim().is_empty()
            || methods.is_empty()
            || methods.iter().any(|m| !METHODS.contains(&m.as_str()))
        {
            bail!("Name, URL prefix and at least one valid method are required");
        }
        let prefix = match_target(prefix)?;
        response.validate()?;
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM rules WHERE prefix=?1 AND id!=?2)",
            params![prefix.trim(), id.unwrap_or(-1)],
            |r| r.get(0),
        )?;
        if exists {
            bail!("URL 前缀已存在，请使用其他前缀");
        }
        let id = if let Some(id) = id {
            let changed=tx.execute("UPDATE rules SET name=?1,prefix=?2,methods=?3,status_code=?4,content_type=?5,response_body=?6 WHERE id=?7",
                params![name.trim(),prefix,serde_json::to_string(methods)?,response.status_code,response.content_type,response.response_body,id])?;
            if changed == 0 {
                bail!("规则已被删除，请刷新后重试");
            }
            tx.execute("DELETE FROM matches WHERE rule_id=?1", [id])?;
            id
        } else {
            tx.execute("INSERT INTO rules(name,prefix,methods,status_code,content_type,response_body) VALUES(?1,?2,?3,?4,?5,?6)",
                params![name.trim(),prefix,serde_json::to_string(methods)?,response.status_code,response.content_type,response.response_body])?;
            tx.last_insert_rowid()
        };
        // substr implements a literal prefix: SQL LIKE would treat % and _ as wildcards.
        for method in methods {
            tx.execute("INSERT OR IGNORE INTO matches SELECT id,?1 FROM requests WHERE method=?2 AND substr(target,1,length(?3))=?3", params![id,method,prefix.trim()])?;
        }
        tx.commit()?;
        self.changed();
        Ok(id)
    }
    pub fn insert(&self, r: &CapturedRequest) -> Result<i64> {
        self.capture(r).map(|(id, _)| id)
    }
    pub fn capture(&self, r: &CapturedRequest) -> Result<(i64, ResponseConfig)> {
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        tx.execute(
            "INSERT INTO requests(time,method,url,peer,headers,body,target) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                r.time,
                r.method,
                r.url,
                r.peer,
                serde_json::to_string(&r.headers)?,
                r.body,
                match_target(&r.url)?
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO matches SELECT ?1,id FROM rules WHERE substr(?2,1,length(prefix))=prefix AND EXISTS(SELECT 1 FROM json_each(rules.methods) WHERE value=?3)",params![id,match_target(&r.url)?,r.method])?;
        let response=tx.query_row("SELECT status_code,content_type,response_body FROM rules WHERE id IN (SELECT rule_id FROM matches WHERE request_id=?1) ORDER BY length(prefix) DESC,id ASC LIMIT 1",[id],|r|Ok(ResponseConfig {status_code:r.get(0)?,content_type:r.get(1)?,response_body:r.get(2)?}));
        let response = match response {
            Ok(response) => response,
            Err(rusqlite::Error::QueryReturnedNoRows) => ResponseConfig::default(),
            Err(e) => return Err(e.into()),
        };
        tx.commit()?;
        self.changed();
        Ok((id, response))
    }
    pub fn delete_rules(&self, ids: &[i64]) -> Result<()> {
        self.delete("rules", ids)
    }
    pub fn delete_requests(&self, ids: &[i64]) -> Result<()> {
        self.delete("requests", ids)
    }
    fn delete(&self, table: &str, ids: &[i64]) -> Result<()> {
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        for id in ids {
            tx.execute(&format!("DELETE FROM {table} WHERE id=?1"), [id])?;
        }
        tx.commit()?;
        self.changed();
        Ok(())
    }
    pub fn counts(&self) -> Result<(usize, usize)> {
        let c = self.conn.lock().unwrap();
        Ok(c.query_row("SELECT count(*),coalesce(sum(NOT EXISTS(SELECT 1 FROM matches m WHERE m.request_id=r.id)),0) FROM requests r", [], |r| Ok((r.get::<_,i64>(0)? as usize,r.get::<_,i64>(1)? as usize)))?)
    }
    pub fn request(&self, id: i64) -> Result<CapturedRequest> {
        let c = self.conn.lock().unwrap();
        let raw = c.query_row(
            "SELECT id,time,method,url,peer,headers,body FROM requests WHERE id=?1",
            [id],
            |r| {
                let headers: String = r.get(5)?;
                Ok((
                    CapturedRequest {
                        id: r.get(0)?,
                        time: r.get(1)?,
                        method: r.get(2)?,
                        url: r.get(3)?,
                        peer: r.get(4)?,
                        headers: vec![],
                        body: r.get(6)?,
                    },
                    headers,
                ))
            },
        )?;
        let (mut request, headers) = raw;
        request.headers = serde_json::from_str(&headers)?;
        Ok(request)
    }
    pub fn page(&self, filter: Filter, page: usize) -> Result<(usize, Vec<RequestSummary>)> {
        let (clause, id) = match filter {
            Filter::All => ("1=1", 0),
            Filter::Unmatched => (
                "NOT EXISTS(SELECT 1 FROM matches m WHERE m.request_id=r.id)",
                0,
            ),
            Filter::Rule(id) => (
                "EXISTS(SELECT 1 FROM matches m WHERE m.request_id=r.id AND m.rule_id=?1)",
                id,
            ),
        };
        let c = self.conn.lock().unwrap();
        let count = c.query_row(
            &format!("SELECT count(*) FROM requests r WHERE {clause} AND ?1=?1"),
            [id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        let mut q = c.prepare(&format!("SELECT id,time,method,url,json_array_length(headers),length(body) FROM requests r WHERE {clause} AND ?1=?1 ORDER BY time DESC,id DESC LIMIT ?2 OFFSET ?3"))?;
        let rows = q
            .query_map(
                params![id, PAGE_SIZE as i64, page.saturating_mul(PAGE_SIZE) as i64],
                |r| {
                    Ok(RequestSummary {
                        id: r.get(0)?,
                        time: r.get(1)?,
                        method: r.get(2)?,
                        url: r.get(3)?,
                        header_count: r.get::<_, i64>(4)? as usize,
                        body_size: r.get::<_, i64>(5)? as usize,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok((count, rows))
    }
}
