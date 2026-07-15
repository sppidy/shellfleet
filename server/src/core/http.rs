use super::{
    model::{CoreEvent, CoreEventKind},
    repository,
};
use crate::AppState;
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{
        IntoResponse, Sse,
        sse::{Event, KeepAlive},
    },
    routing::get,
};
use futures_util::stream;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/fleet", get(fleet))
        .route("/events", get(events))
}

async fn fleet(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match repository::list_fleet(&state.db, crate::now_unix()).await {
        Ok(fleet) => Json(fleet).into_response(),
        Err(error) => {
            tracing::error!(%error, "durable fleet query failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "fleet repository unavailable",
            )
                .into_response()
        }
    }
}

async fn events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.core_events.subscribe();
    let output = stream::unfold(receiver, |mut receiver| async move {
        match receiver.recv().await {
            Ok(event) => Some((Ok(to_sse(event)), receiver)),
            Err(RecvError::Lagged(_)) => Some((
                Ok(to_sse(CoreEvent {
                    id: 0,
                    kind: CoreEventKind::ResyncRequired,
                    agent_id: None,
                    observed_at: crate::now_unix(),
                })),
                receiver,
            )),
            Err(RecvError::Closed) => None,
        }
    });
    Sse::new(output).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

fn to_sse(event: CoreEvent) -> Event {
    Event::default()
        .id(event.id.to_string())
        .event("fleet")
        .json_data(event)
        .expect("CoreEvent contains only serializable fields")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppState;
    use axum::Router;
    use std::sync::Arc;

    #[test]
    fn core_routes_construct_with_application_state() {
        let _: Router<Arc<AppState>> = routes();
    }
}
