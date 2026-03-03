use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub async fn ping() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ping() {
        let Json(resp) = ping().await;
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.version, env!("CARGO_PKG_VERSION"));
    }
}
