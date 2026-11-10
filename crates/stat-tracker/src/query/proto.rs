//! Request and response lines, and the answer the daemon computes.

use serde::{Deserialize, Serialize};

use crate::games::aggregate::{Aggregates, GameFilter, aggregate_filtered, filter_games};
use crate::games::model::Game;

/// Protocol version. A client and a daemon of different versions do not
/// talk; the client falls back to `live_snapshot.json`.
pub const PROTO: u32 = 1;

/// Longest request line the daemon reads.
pub const MAX_REQUEST: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "q", rename_all = "snake_case")]
pub enum Request {
    Hello,
    /// The stored-data revision. Cheap: no store read.
    Rev,
    /// One page's totals for a filter (Overview, Heroes, Maps, Seasons).
    Totals {
        filter: GameFilter,
    },
    /// The games a filter keeps, newest first (the Games list).
    Games {
        filter: GameFilter,
    },
}

/// A request line: `{"v":1,"q":"totals","filter":{...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestLine {
    pub v: u32,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Response {
    Hello {
        proto: u32,
        version: String,
        rev: u64,
    },
    Rev {
        rev: u64,
    },
    Totals {
        rev: u64,
        totals: Aggregates,
    },
    Games {
        rev: u64,
        games: Vec<Game>,
    },
    Err {
        code: String,
        msg: String,
    },
}

impl Response {
    pub fn err(code: &str, msg: impl Into<String>) -> Self {
        Response::Err {
            code: code.into(),
            msg: msg.into(),
        }
    }
}

/// Parse one request line; a bad line or another protocol version is an
/// `Err` response to send back.
pub fn parse_request(line: &[u8]) -> Result<Request, Response> {
    let parsed: RequestLine =
        serde_json::from_slice(line).map_err(|e| Response::err("bad_request", e.to_string()))?;
    if parsed.v != PROTO {
        return Err(Response::err(
            "unsupported_proto",
            format!("this tracker speaks {PROTO}"),
        ));
    }
    Ok(parsed.request)
}

/// The daemon's answer over `games` (the list `games_from_snapshot` gives)
/// at revision `rev`. The same functions the pages call.
pub fn answer(request: &Request, rev: u64, games: &[Game]) -> Response {
    match request {
        Request::Hello => Response::Hello {
            proto: PROTO,
            version: crate::version::APP_VERSION.to_string(),
            rev,
        },
        Request::Rev => Response::Rev { rev },
        Request::Totals { filter } => Response::Totals {
            rev,
            totals: aggregate_filtered(games, filter),
        },
        Request::Games { filter } => Response::Games {
            rev,
            games: filter_games(games, filter).into_iter().cloned().collect(),
        },
    }
}

/// One response line (newline-terminated JSON).
pub fn encode(response: &Response) -> Vec<u8> {
    let mut out = serde_json::to_vec(response).unwrap_or_else(|e| {
        serde_json::to_vec(&Response::err("internal", e.to_string())).unwrap_or_default()
    });
    out.push(b'\n');
    out
}
