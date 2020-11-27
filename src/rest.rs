//! `warp` integration for serving over HTTP.

use std::borrow::Cow;

use fallible_collections::{tryformat, TryReserveError};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use warp::body::BodyDeserializeError;
use warp::{Filter, Rejection, Reply};

use crate::Engine;

/// Converts the container engine into a [`warp`](https://docs.rs/warp) REST filter.
pub fn to_filter(svc: Engine) -> impl Filter<Extract = impl Reply> + Clone + 'static {
    let container_path = warp::path!("containers" / String);
    let engine = warp::any().map(move || svc.clone());

    let create = warp::put()
        .and(engine.clone())
        .and(container_path)
        .and_then(move |eng: Engine, name: String| async move {
            if let Err(e) = eng.create(&name).await {
                eprintln!("error creating container: {}", e);
                Err(warp::reject::custom(EngineError(e)))
            } else {
                Ok(warp::reply())
            }
        });

    let delete = warp::delete()
        .and(engine.clone())
        .and(container_path)
        .and_then(move |eng: Engine, name: String| async move {
            if let Err(e) = eng.delete(&name).await {
                eprintln!("error deleting container: {}", e);
                Err(warp::reject::custom(EngineError(e)))
            } else {
                Ok(warp::reply())
            }
        });

    let modify = warp::put()
        .and(engine.clone())
        .and(warp::path!("containers" / String / "status"))
        .and(warp::body::json())
        .and_then(move |eng: Engine, name: String, body: Modify| async move {
            let result = match body.state {
                State::Paused => eng.pause(&name).await,
                State::Running => eng.resume(&name).await,
            };

            if let Err(e) = result {
                eprintln!("error modifying container state: {}", e);
                Err(warp::reject::custom(EngineError(e)))
            } else {
                Ok(warp::reply())
            }
        });

    let state = warp::get().and(engine).and(container_path).and_then(
        move |eng: Engine, name: String| async move {
            match eng.state(&name).await {
                Ok(state) => Ok(warp::reply::json(&state)),
                Err(e) => {
                    eprintln!("error retrieving container state: {}", e);
                    Err(warp::reject::custom(EngineError(e)))
                }
            }
        },
    );

    (create.or(delete).or(modify).or(state)).recover(handle_rejection)
}

/// A list of possible container state transitions.
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum State {
    /// A state transition to pause a running container.
    Paused,
    /// A state transition to resume a paused container.
    Running,
}

/// A JSON body for the pause/resume requests.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Modify {
    /// The state transition to be applied in-place.
    state: State,
}

/// Custom `warp` rejection wrapping a container engine error.
#[derive(Debug)]
struct EngineError(anyhow::Error);

impl warp::reject::Reject for EngineError {}

/// Custom `warp` rejection wrapping an out-of-memory error.
#[derive(Debug)]
struct OomError(TryReserveError);

impl warp::reject::Reject for OomError {}

/// A JSON error message response.
#[derive(Serialize)]
struct ErrorMsg<'a> {
    code: u16,
    message: Cow<'a, str>,
}

/// Converts the `warp::Rejection` into a JSON response with a status code and error message.
///
/// Returns `Err` if an out-of-memory error occurred during the conversion, or an unhandled
/// rejection case was encountered.
async fn handle_rejection(err: Rejection) -> Result<impl Reply, Rejection> {
    let code;
    let message;

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = Cow::from("Container not found");
    } else if let Some(EngineError(e)) = err.find::<EngineError>() {
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = tryformat!(64, "{}", e)
            .map(Cow::from)
            .map_err(|e| warp::reject::custom(OomError(e)))?;
    } else if let Some(e) = err.find::<BodyDeserializeError>() {
        code = StatusCode::BAD_REQUEST;
        message = tryformat!(256, "{}", e)
            .map(Cow::from)
            .map_err(|e| warp::reject::custom(OomError(e)))?;
    } else {
        eprintln!("unhandled rejection: {:?}", err);
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = Cow::from("UNHANDLED_REJECTION");
    }

    let json = warp::reply::json(&ErrorMsg {
        code: code.as_u16(),
        message,
    });

    Ok(warp::reply::with_status(json, code))
}
