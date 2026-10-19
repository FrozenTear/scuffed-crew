use axum::extract::State;
use axum::http::StatusCode;

use crate::state::AppState;

/// Liveness. Does not touch the database.
pub async fn health() -> StatusCode {
    StatusCode::OK
}

/// Readiness. 200 only when SurrealDB answers a trivial query, 503 otherwise.
///
/// The body is empty. The error is logged and is not returned to the caller.
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    ready_status(state.db.ping().await)
}

pub(crate) fn ready_status(result: scuffed_db::DbResult<()>) -> StatusCode {
    match result {
        Ok(()) => StatusCode::OK,
        Err(err) => {
            tracing::warn!(error = %err, "readiness check failed");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_db::DbError;

    #[test]
    fn ready_status_is_503_when_the_probe_fails() {
        assert_eq!(ready_status(Ok(())), StatusCode::OK);
        assert_eq!(
            ready_status(Err(DbError::Timeout)),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
