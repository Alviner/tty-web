//! Static file serving with compile-time embedded frontend assets.
//!
//! All files under `frontend/` are embedded into the binary via
//! [`rust_embed`]. MIME types are detected automatically.

use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Frontend assets embedded at compile time.
#[derive(RustEmbed)]
#[folder = "frontend/"]
struct Assets;

pub async fn index() -> impl IntoResponse {
    serve_file("index.html")
}

pub async fn static_file(
    Path(path): Path<String>,
) -> impl IntoResponse {
    serve_file(&path)
}

fn serve_file(path: &str) -> Response {
    let Some(file) = Assets::get(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    let content_type = HeaderValue::from_str(&mime)
        .unwrap_or(HeaderValue::from_static("application/octet-stream"));

    let mut response =
        Response::new(Body::from(file.data.to_vec()));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_html() {
        let response = serve_file("index.html");
        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content-type header");
        assert!(
            ct.to_str().unwrap().contains("text/html"),
            "expected text/html, got: {:?}",
            ct
        );
    }

    #[test]
    fn test_not_found() {
        let response = serve_file("nonexistent_file.xyz");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
