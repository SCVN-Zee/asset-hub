//! A throwaway HTTP server on localhost for the tests that need a GitHub API stand-in.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self { status, headers: vec![("Content-Type".into(), "application/json".into())], body: body.as_bytes().to_vec() }
    }

    pub fn bytes(body: Vec<u8>) -> Self {
        Self { status: 200, headers: Vec::new(), body }
    }
}

/// What the server saw: the request path and the lower-cased header names.
pub type Request = (String, HashMap<String, String>);

/// Serves `handler` on a free port until the test ends; answers the base URL.
pub async fn serve(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let base = format!("http://{}", listener.local_addr().expect("an address"));
    let handler = Arc::new(handler);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else { return };
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut chunk = [0u8; 4096];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&raw).into_owned();
                let mut lines = text.split("\r\n");
                let path = lines.next().and_then(|l| l.split(' ').nth(1)).unwrap_or("/").to_owned();
                let headers = lines.filter_map(|l| l.split_once(": ")).map(|(k, v)| (k.to_lowercase(), v.to_owned())).collect();
                let reply = handler(&(path, headers));
                let mut head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len());
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&reply.body).await;
            });
        }
    });
    base
}
