use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::TryStreamExt;
use reqwest::Response as ReqwestResponse;

/// Creates a streaming Axum HTTP response that directly pipes upstream chunks to the client
/// without buffering the whole body into memory.
pub fn create_sse_response(upstream_res: ReqwestResponse) -> Response {
    let status = upstream_res.status();
    let stream = upstream_res
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let body = Body::from_stream(stream);

    Response::builder()
        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK))
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream; charset=utf-8"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header(header::CONNECTION, HeaderValue::from_static("keep-alive"))
        .header("X-Accel-Buffering", HeaderValue::from_static("no"))
        .body(body)
        .unwrap_or_else(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to construct streaming response",
            )
                .into_response()
        })
}
