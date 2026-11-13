use crate::store::{CapturedRequest, Header, Store};
use anyhow::Result;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    response::IntoResponse,
};
use std::{
    net::{SocketAddr, TcpListener},
    thread,
};
use tokio::sync::oneshot;

pub const MAX_BODY: usize = 16 * 1024 * 1024;
pub struct Listener {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
    pub address: SocketAddr,
}
impl Listener {
    pub fn start(store: Store, address: SocketAddr) -> Result<Self> {
        // Bind synchronously so address conflicts appear immediately in the toolbar.
        let listener = TcpListener::bind(address)?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let runtime = tokio::runtime::Runtime::new()?;
        let (stop, stopped) = oneshot::channel();
        let thread = thread::spawn(move || {
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("bound listener");
                let app = Router::new().fallback(capture).with_state(store);
                let _ = axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await;
            })
        });
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
            address,
        })
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        } /* Do not block UI on slow clients. */
        self.thread.take();
    }
}
async fn capture(
    State(store): State<Store>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request<Body>,
) -> impl IntoResponse {
    let (parts, body) = request.into_parts();
    let url = if parts.uri.scheme().is_some() {
        parts.uri.to_string()
    } else {
        format!(
            "http://{}{}",
            parts
                .headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("localhost"),
            parts.uri
        )
    };
    let body = match to_bytes(body, MAX_BODY).await {
        Ok(b) => b.to_vec(),
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                "Request body exceeds 16 MiB or could not be read",
            )
                .into_response();
        }
    };
    let request = CapturedRequest {
        id: 0,
        time: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        method: parts.method.to_string(),
        url,
        peer: peer.to_string(),
        headers: parts
            .headers
            .iter()
            .map(|(k, v)| Header::new(k.to_string(), v.as_bytes()))
            .collect(),
        body,
    };
    match tokio::task::spawn_blocking(move || store.capture(&request)).await {
        Ok(Ok((_, response))) => {
            let mut result = (
                StatusCode::from_u16(response.status_code)
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                response.response_body,
            )
                .into_response();
            if let Ok(value) = response.content_type.parse() {
                result
                    .headers_mut()
                    .insert(axum::http::header::CONTENT_TYPE, value);
            }
            result
        }
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to persist request",
        )
            .into_response(),
    }
}
