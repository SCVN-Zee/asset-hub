//! The `assethub-thumb://` scheme: the page loads thumbnails straight from the cache folder.
//!
//! A canvas stays untainted (so `getImageData` works) only when the response carries
//! `Access-Control-Allow-Origin` and the `<img>` sets `crossOrigin = 'anonymous'`; every response
//! here carries it, including the 404.

use percent_encoding::percent_decode_str;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime};

use crate::cache::thumbs::{ThumbnailStore, THUMB_SCHEME};
use crate::state::AppState;

fn reply(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        // The URL carries a version, and the file is on local disk: nothing to keep a copy of.
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .expect("a response of fixed, valid parts")
}

/// The thumbnail a request names (`assethub-thumb://t/<name>?v=<version>`), or 404.
pub async fn serve(store: &ThumbnailStore, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let name = percent_decode_str(request.uri().path().trim_start_matches('/')).decode_utf8().ok();
    match store.read(name.as_deref().unwrap_or_default()).await {
        Some(found) => reply(StatusCode::OK, found.content_type, found.bytes),
        None => reply(StatusCode::NOT_FOUND, "text/plain", Vec::new()),
    }
}

/// Registers the scheme on the app builder. The store is looked up per request, because the app
/// state exists only after setup.
pub fn register<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(THUMB_SCHEME, |context, request, responder| {
        let app = context.app_handle().clone();
        tauri::async_runtime::spawn(async move {
            let response = match app.try_state::<AppState>() {
                Some(state) => serve(&state.caches.thumbs, &request).await,
                None => reply(StatusCode::SERVICE_UNAVAILABLE, "text/plain", Vec::new()),
            };
            responder.respond(response);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::thumbs::tests::PNG;
    use crate::cache::CacheConfig;

    fn get(uri: &str) -> Request<Vec<u8>> {
        Request::builder().uri(uri).body(Vec::new()).unwrap()
    }

    fn cors(response: &Response<Vec<u8>>) -> Option<&str> {
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).and_then(|v| v.to_str().ok())
    }

    #[tokio::test]
    async fn serves_a_thumbnail_with_its_type_and_the_cors_header() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThumbnailStore::new(CacheConfig::new(dir.path().to_owned(), 0));
        store.save("sha1", PNG).await.unwrap();
        let url = store.get_many(&["sha1".into()]).await["sha1"].clone();

        let response = serve(&store, &get(&url)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(cors(&response), Some("*"));
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(response.body()[..4], [0x89, b'P', b'N', b'G']);
    }

    #[tokio::test]
    async fn a_missing_thumbnail_is_404_and_still_carries_the_cors_header() {
        let dir = tempfile::tempdir().unwrap();
        let store = ThumbnailStore::new(CacheConfig::new(dir.path().to_owned(), 0));
        let response = serve(&store, &get("assethub-thumb://t/none?v=0")).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(cors(&response), Some("*"));
    }

    #[tokio::test]
    async fn a_name_that_leaves_the_thumbnail_folder_is_404() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.img"), [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();
        let store = ThumbnailStore::new(CacheConfig::new(dir.path().to_owned(), 0));
        for uri in ["assethub-thumb://t/..%2Fsecret?v=0", "assethub-thumb://t/../secret", "assethub-thumb://t/%2e%2e%2fsecret"] {
            let response = serve(&store, &get(uri)).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
            assert_eq!(cors(&response), Some("*"));
        }
    }

    #[test]
    fn the_scheme_registers_on_a_builder() {
        let _ = register(tauri::test::mock_builder());
    }
}
