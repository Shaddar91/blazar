//! Custom extractors.
//!
//! `JsonOrSilent<T>` wraps `axum::Json` so that a malformed/incomplete body
//! (anything that would normally produce an `HTTP 422 Unprocessable Entity`
//! from serde) is converted into a silent `HTTP 204 No Content`. This lets
//! us swallow drive-by garbage and stale frontend payloads without leaking
//! schema details, and matches the frontend's existing
//! `if (res.status === 204) return ok` branch.

use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::de::DeserializeOwned;

pub struct JsonOrSilent<T>(pub T);

impl<T, S> FromRequest<S> for JsonOrSilent<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(JsonOrSilent(value)),
            Err(rej) => {
                tracing::info!(rejection = %rej, "malformed body — silent 204");
                Err(StatusCode::NO_CONTENT.into_response())
            }
        }
    }
}

impl<T> std::ops::Deref for JsonOrSilent<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for JsonOrSilent<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
