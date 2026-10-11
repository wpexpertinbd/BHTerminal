//! Bridges a VNC server's raw TCP to a WebSocket the webview can open.
//!
//! VNC speaks RFB over a plain socket; a webview cannot open one. noVNC — the
//! same client Proxmox and OpenStack ship — renders RFB to a canvas and
//! expects a WebSocket, so this is the adapter between the two (the job
//! `websockify` does). Using it means the encodings, auth and clipboard
//! handling are a mature implementation rather than one we wrote.
//!
//! ⚠️ SECURITY. A naive bridge is an open proxy: anything that can reach the
//! port could ask it to connect anywhere, turning the app into an SSRF
//! gadget for the whole local network. So:
//!   * it listens on 127.0.0.1 only, never 0.0.0.0;
//!   * the port is ephemeral, not a known number;
//!   * the CLIENT NEVER NAMES A TARGET. It presents a token, and the token
//!     was registered by us against one specific host:port;
//!   * tokens are single-use and removed as soon as they are redeemed, so a
//!     leaked URL cannot be replayed.

use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::Message;

type Pending = Arc<Mutex<HashMap<String, (String, u16)>>>;

#[derive(Default)]
pub struct VncBridge {
    port: Mutex<Option<u16>>,
    pending: Pending,
}

impl VncBridge {
    /// Registers a one-shot token for this target and returns the URL the
    /// webview should open. Starts the listener on first use.
    pub async fn prepare(&self, host: String, port: u16) -> Result<String, String> {
        let listen_port = self.ensure_started().await?;
        let token: String = {
            let mut rng = rand::thread_rng();
            (0..32)
                .map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rng.gen_range(0..36)]))
                .collect()
        };
        self.pending.lock().unwrap().insert(token.clone(), (host, port));
        Ok(format!("ws://127.0.0.1:{listen_port}/vnc?token={token}"))
    }

    async fn ensure_started(&self) -> Result<u16, String> {
        if let Some(p) = *self.port.lock().unwrap() {
            return Ok(p);
        }
        // Loopback only. Binding 0.0.0.0 here would expose every machine the
        // user can reach to anyone on their network.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("couldn't start the VNC bridge: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        *self.port.lock().unwrap() = Some(port);

        let pending = self.pending.clone();
        tauri::async_runtime::spawn(async move {
            while let Ok((stream, addr)) = listener.accept().await {
                // Belt and braces: the OS already restricts us to loopback,
                // but refuse anything that isn't.
                if !addr.ip().is_loopback() {
                    continue;
                }
                let pending = pending.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = serve(stream, pending).await;
                });
            }
        });
        Ok(port)
    }
}

// The handshake callback must return tungstenite's ErrorResponse — a whole
// HTTP response — so the "large Err variant" lint has nothing we can act on
// without boxing a type the library hands us.
#[allow(clippy::result_large_err)]
async fn serve(stream: TcpStream, pending: Pending) -> Result<(), String> {
    // The token is read during the handshake and redeemed immediately, so a
    // replay of the same URL finds nothing.
    let claimed: Arc<Mutex<Option<(String, u16)>>> = Arc::new(Mutex::new(None));
    let claimed_cb = claimed.clone();

    let ws = tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let query = req.uri().query().unwrap_or("");
        let token = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("token="))
            .unwrap_or("");
        match pending.lock().unwrap().remove(token) {
            Some(target) => {
                *claimed_cb.lock().unwrap() = Some(target);
                Ok(resp)
            }
            None => {
                let mut err = ErrorResponse::new(Some("unknown or already-used token".into()));
                *err.status_mut() = StatusCode::FORBIDDEN;
                Err(err)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    let (host, port) = claimed.lock().unwrap().take().ok_or("no target")?;

    let upstream = tokio::time::timeout(
        Duration::from_secs(20),
        TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| "timed out connecting to the VNC server".to_string())?
    .map_err(|e| format!("couldn't reach {host}:{port}: {e}"))?;
    let _ = upstream.set_nodelay(true); // interactive: latency beats throughput

    let (mut ws_tx, mut ws_rx) = ws.split();
    let (mut up_rx, mut up_tx) = upstream.into_split();

    // webview → VNC server
    let to_server = async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                Message::Binary(b) => {
                    if up_tx.write_all(&b).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = up_tx.shutdown().await;
    };

    // VNC server → webview
    let to_client = async move {
        let mut buf = vec![0u8; 32 * 1024];
        loop {
            match up_rx.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = ws_tx.close().await;
    };

    tokio::join!(to_server, to_client);
    Ok(())
}

pub type SharedVnc = Arc<VncBridge>;
