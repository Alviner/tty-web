use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

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
