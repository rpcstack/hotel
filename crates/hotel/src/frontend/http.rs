//! The frontend's public HTTP/JSON API, the same routes, parameters, status
//! codes and JSON shapes as DeathStarBench's Go hotel frontend
//! (`hotelReservation/services/frontend`), so one load generator drives both.
//!
//! Every request enters the RPC stack the way `hotel_client_bench` does: this
//! module creates the request's root context (API name, per-route SLO from
//! `config/hotel.json` `frontend.sloMs`, deadline = now + SLO, stamped as the
//! gRPC `grpc-timeout`; see `app_utils::root_context`), attaches it to a
//! `FrontendClient` request and calls the frontend service. That client is
//! connected in process to the very same `FrontendServer` that serves gRPC on
//! the gRPC port: client hooks, the frontend's server hooks and its child RPCs
//! all run as they do for a remote gRPC client; only TCP and HTTP/2 framing are
//! skipped. The handler task is spawned with the priority in the request's
//! scheduling header, as Hyper does for each HTTP/2 stream.
//!
//! Not supported by this port (answered 501, never faked): the owner routes
//! `POST /admin/{rate,profile,capacity,hotel}` (the services have no write
//! RPCs) and `/review`, `/restaurants`, `/museums`, `/cinema` (review is a
//! frontend stub, there is no attractions service).

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;

use app_utils::root_context::RootContext;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, StatusCode};
use serde::Serialize;
use tonic::body::BoxBody;
use tonic::codegen::Service;
use tonic::Code;

use crate::config::HttpSlosMs;
use crate::server::hotel_tonic::frontend::{
    self, frontend_client::FrontendClient, frontend_server::FrontendServer,
};
use crate::server::FrontendImpl;

/// Header carrying the opaque scheduling priority (`rpcstack_wire::SCHEDULING_HEADER`).
const SCHEDULING_HEADER: &str = "x-rpcstack-scheduling";

/// The frontend gRPC service called without a socket. Spawns the handler as a
/// task with the priority of the request's scheduling header, like Hyper's
/// HTTP/2 server does for each stream (infrastructure priority when absent).
#[derive(Clone)]
pub struct InProcess(FrontendServer<FrontendImpl>);

impl InProcess {
    pub fn new(server: FrontendServer<FrontendImpl>) -> Self {
        Self(server)
    }
}

impl Service<http::Request<BoxBody>> for InProcess {
    type Response = http::Response<BoxBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut TaskContext<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<BoxBody>) -> Self::Future {
        let priority = req
            .headers()
            .get(SCHEDULING_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(tokio::task::TaskPriority::new)
            .unwrap_or_else(|| tokio::task::meta_for_unannotated_spawn(None));
        let handle = tokio::task::spawn_with_prio(self.0.call(req), priority);
        Box::pin(async move {
            Ok(match handle.await {
                Ok(Ok(response)) => response,
                Ok(Err(never)) => match never {},
                Err(e) => {
                    tonic::Status::internal(format!("frontend handler failed: {e}")).to_http()
                }
            })
        })
    }
}

pub type LocalClient = FrontendClient<InProcess>;

pub struct HttpFrontend {
    client: LocalClient,
    slo: HttpSlosMs,
}

impl HttpFrontend {
    pub fn new(client: LocalClient, slo: HttpSlosMs) -> Self {
        Self { client, slo }
    }
}

pub async fn serve(addr: SocketAddr, state: Arc<HttpFrontend>) -> Result<(), hyper::Error> {
    let make = make_service_fn(move |_conn| {
        let state = state.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(route(&state, req).await) }
            }))
        }
    });
    hyper::Server::bind(&addr).serve(make).await
}

// ---------------------------------------------------------------- responses

fn reply(status: StatusCode, content_type: &'static str, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("access-control-allow-origin", "*")
        .body(Body::from(body))
        .unwrap()
}

/// Go's `http.Error`: plain text and a trailing newline.
fn text_error(status: StatusCode, message: &str) -> Response<Body> {
    reply(status, "text/plain; charset=utf-8", format!("{message}\n"))
}

/// Go's `json.NewEncoder(w).Encode(v)`: compact JSON and a trailing newline.
fn json<T: Serialize>(value: &T) -> Response<Body> {
    let mut body = serde_json::to_string(value).expect("serialize response");
    body.push('\n');
    reply(StatusCode::OK, "application/json", body)
}

#[derive(Serialize)]
struct Message {
    message: String,
}

fn message(text: &str) -> Response<Body> {
    json(&Message {
        message: text.to_string(),
    })
}

// GeoJSON, with the key order and float32 coordinates Go's encoder produces.
#[derive(Serialize)]
struct FeatureCollection {
    features: Vec<Feature>,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Feature {
    geometry: Geometry,
    id: String,
    properties: Properties,
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Geometry {
    coordinates: [f32; 2],
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Properties {
    name: String,
    phone_number: String,
}

fn geo_json(hotels: Vec<frontend::Hotel>) -> Response<Body> {
    json(&FeatureCollection {
        features: hotels
            .into_iter()
            .map(|h| Feature {
                geometry: Geometry {
                    coordinates: [h.lon, h.lat],
                    kind: "Point",
                },
                id: h.id,
                properties: Properties {
                    name: h.name,
                    phone_number: h.phone_number,
                },
                kind: "Feature",
            })
            .collect(),
        kind: "FeatureCollection",
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Rates {
    hotel_id: String,
    rate_plans: Vec<RatePlan>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RatePlan {
    code: String,
    in_date: String,
    out_date: String,
    room_type: RoomType,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomType {
    bookable_rate: f64,
    code: String,
    room_description: String,
    total_rate: f64,
    total_rate_inclusive: f64,
}

/// The HTTP status of a failed RPC. Requests the policy rejects or aborts
/// (`/EarlyReturn`, admission) come back as DEADLINE_EXCEEDED, UNAVAILABLE,
/// RESOURCE_EXHAUSTED or ABORTED; Go's frontend answers every error 500.
fn grpc_error(status: &tonic::Status) -> Response<Body> {
    let code = match status.code() {
        Code::InvalidArgument => StatusCode::BAD_REQUEST,
        Code::NotFound => StatusCode::NOT_FOUND,
        Code::AlreadyExists | Code::FailedPrecondition => StatusCode::CONFLICT,
        Code::DeadlineExceeded => StatusCode::GATEWAY_TIMEOUT,
        Code::Unavailable | Code::ResourceExhausted | Code::Aborted => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    text_error(code, status.message())
}

// ------------------------------------------------------------------ routing

/// The first value of each query parameter, as Go's `URL.Query().Get`.
struct Query(Vec<(String, String)>);

impl Query {
    fn of(req: &Request<Body>) -> Self {
        Query(
            form_urlencoded::parse(req.uri().query().unwrap_or("").as_bytes())
                .into_owned()
                .collect(),
        )
    }

    fn get(&self, key: &str) -> &str {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
}

async fn route(state: &HttpFrontend, req: Request<Body>) -> Response<Body> {
    let query = Query::of(&req);
    match req.uri().path() {
        "/hotels" => hotels(state, &query).await,
        "/recommendations" => recommendations(state, &query).await,
        "/user" => user(state, &query).await,
        "/reservation" => reservation(state, &query).await,
        "/rates" => rates(state, &query).await,
        "/admin/rate" | "/admin/profile" | "/admin/capacity" | "/admin/hotel" => {
            if req.method() != Method::POST {
                let mut r = text_error(StatusCode::METHOD_NOT_ALLOWED, "Please use POST");
                r.headers_mut().insert("allow", "POST".parse().unwrap());
                return r;
            }
            text_error(
                StatusCode::NOT_IMPLEMENTED,
                "owner writes are not supported by the Rust hotel port (its services have no write RPCs)",
            )
        }
        "/review" | "/restaurants" | "/museums" | "/cinema" => text_error(
            StatusCode::NOT_IMPLEMENTED,
            "not supported by the Rust hotel port (review is a stub, there is no attractions service)",
        ),
        _ => text_error(StatusCode::NOT_FOUND, "404 page not found"),
    }
}

/// Create the request's root context, as `hotel_client_bench` does, and call.
/// `api` is the API name; `slo_ms` comes from the app config.
macro_rules! call_with_root {
    ($state:expr, $api:expr, $slo_ms:expr, $method:ident, $msg:expr) => {{
        let ctx = RootContext::new($api, Duration::from_millis($slo_ms));
        let request = ctx.attach(tonic::Request::new($msg));
        let mut client = $state.client.clone();
        match client.$method(request).await {
            Ok(response) => response.into_inner(),
            Err(status) => return grpc_error(&status),
        }
    }};
}

fn bad_request(message: &str) -> Response<Body> {
    text_error(StatusCode::BAD_REQUEST, message)
}

/// `strconv.ParseFloat(s, 32)` then `float32`: Go's rounding to float32, errors ignored (0).
fn parse_f32(s: &str) -> f64 {
    s.parse::<f64>().map(|v| v as f32 as f64).unwrap_or(0.0)
}

fn parse_f64(s: &str) -> f64 {
    s.parse::<f64>().unwrap_or(0.0)
}

/// YYYY-MM-DD naming a real day. (Go's `/hotels` accepts any text; the Rust
/// reservation service would panic on a bad date, so both routes validate.)
fn valid_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// `GET /hotels?inDate=&outDate=&lat=&lon=[&locale=]`: GeoJSON of the available hotels nearby.
async fn hotels(state: &HttpFrontend, q: &Query) -> Response<Body> {
    let (in_date, out_date) = (q.get("inDate"), q.get("outDate"));
    if in_date.is_empty() || out_date.is_empty() {
        return bad_request("Please specify inDate/outDate params");
    }
    let (lat, lon) = (q.get("lat"), q.get("lon"));
    if lat.is_empty() || lon.is_empty() {
        return bad_request("Please specify location params");
    }
    if !valid_date(in_date) || !valid_date(out_date) {
        return bad_request("Please check inDate/outDate format (YYYY-MM-DD)");
    }
    let locale = q.get("locale");
    let response = call_with_root!(
        state,
        "Search",
        state.slo.search,
        handle_search,
        frontend::SearchRequest {
            lat: parse_f32(lat),
            lon: parse_f32(lon),
            in_date: in_date.to_string(),
            out_date: out_date.to_string(),
            locale: (!locale.is_empty()).then(|| locale.to_string()),
        }
    );
    geo_json(response.hotels)
}

/// `GET /recommendations?require=dis|rate|price&lat=&lon=[&locale=]`.
async fn recommendations(state: &HttpFrontend, q: &Query) -> Response<Body> {
    let (lat, lon) = (q.get("lat"), q.get("lon"));
    if lat.is_empty() || lon.is_empty() {
        return bad_request("Please specify location params");
    }
    let require = q.get("require");
    if !matches!(require, "dis" | "rate" | "price") {
        return bad_request("Please specify require params");
    }
    let locale = q.get("locale");
    let response = call_with_root!(
        state,
        "Recommendation",
        state.slo.recommendation,
        handle_recommendation,
        frontend::RecommendationRequest {
            require: require.to_string(),
            lat: parse_f64(lat),
            lon: parse_f64(lon),
            locale: (!locale.is_empty()).then(|| locale.to_string()),
        }
    );
    geo_json(response.hotels)
}

/// `GET /user?username=&password=`.
async fn user(state: &HttpFrontend, q: &Query) -> Response<Body> {
    let (username, password) = (q.get("username"), q.get("password"));
    if username.is_empty() || password.is_empty() {
        return bad_request("Please specify username and password");
    }
    let response = call_with_root!(
        state,
        "User",
        state.slo.user,
        handle_user,
        frontend::UserRequest {
            username: username.to_string(),
            password: password.to_string(),
        }
    );
    message(if response.correct {
        "Login successfully!"
    } else {
        "Failed. Please check your username and password. "
    })
}

/// `GET /reservation?inDate=&outDate=&hotelId=&customerName=&username=&password=[&number=]`.
async fn reservation(state: &HttpFrontend, q: &Query) -> Response<Body> {
    let (in_date, out_date) = (q.get("inDate"), q.get("outDate"));
    if in_date.is_empty() || out_date.is_empty() {
        return bad_request("Please specify inDate/outDate params");
    }
    if !valid_date(in_date) || !valid_date(out_date) {
        return bad_request("Please check inDate/outDate format (YYYY-MM-DD)");
    }
    let hotel_id = q.get("hotelId");
    if hotel_id.is_empty() {
        return bad_request("Please specify hotelId params");
    }
    let customer = q.get("customerName");
    if customer.is_empty() {
        return bad_request("Please specify customerName params");
    }
    let (username, password) = (q.get("username"), q.get("password"));
    if username.is_empty() || password.is_empty() {
        return bad_request("Please specify username and password");
    }
    // Go: `strconv.Atoi`, errors ignored (0).
    let rooms: i32 = q.get("number").parse().unwrap_or(0);
    let response = call_with_root!(
        state,
        "Reservation",
        state.slo.reservation,
        handle_reservation,
        frontend::ReservationRequest {
            username: username.to_string(),
            password: password.to_string(),
            customer: customer.to_string(),
            hotels: vec![hotel_id.to_string()],
            in_date: in_date.to_string(),
            out_date: out_date.to_string(),
            num_rooms: rooms.max(0) as u32,
        }
    );
    message(if !response.user_correct {
        "Failed. Please check your username and password. "
    } else if response.hotels.is_empty() {
        "Failed. Already reserved. "
    } else {
        "Reserve successfully!"
    })
}

/// `GET /rates?hotelId=`: the hotel's rate plans.
async fn rates(state: &HttpFrontend, q: &Query) -> Response<Body> {
    let hotel_id = q.get("hotelId");
    if hotel_id.is_empty() {
        return bad_request("Please specify hotelId params");
    }
    let response = call_with_root!(
        state,
        "Rates",
        state.slo.rates,
        handle_rates,
        frontend::RatesRequest {
            hotel_id: hotel_id.to_string(),
        }
    );
    json(&Rates {
        hotel_id: hotel_id.to_string(),
        rate_plans: response
            .rate_plans
            .into_iter()
            .map(|p| {
                let room = p.room_type.unwrap_or_default();
                RatePlan {
                    code: p.code,
                    in_date: p.in_date,
                    out_date: p.out_date,
                    room_type: RoomType {
                        bookable_rate: room.bookable_rate,
                        code: room.code,
                        room_description: room.room_description,
                        total_rate: room.total_rate,
                        total_rate_inclusive: room.total_rate_inclusive,
                    },
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn body_of(r: Response<Body>) -> String {
        String::from_utf8(hyper::body::to_bytes(r.into_body()).await.unwrap().to_vec()).unwrap()
    }

    #[tokio::test]
    async fn geo_json_matches_the_go_encoding() {
        let r = geo_json(vec![frontend::Hotel {
            name: "Hilton".into(),
            id: "7".into(),
            phone_number: "(415) 1".into(),
            lat: 37.7867,
            lon: -122.4112,
        }]);
        assert_eq!(
            body_of(r).await,
            "{\"features\":[{\"geometry\":{\"coordinates\":[-122.4112,37.7867],\"type\":\"Point\"},\
             \"id\":\"7\",\"properties\":{\"name\":\"Hilton\",\"phone_number\":\"(415) 1\"},\
             \"type\":\"Feature\"}],\"type\":\"FeatureCollection\"}\n"
        );
        assert_eq!(
            body_of(geo_json(vec![])).await,
            "{\"features\":[],\"type\":\"FeatureCollection\"}\n"
        );
    }

    #[tokio::test]
    async fn message_is_a_single_field_object() {
        assert_eq!(
            body_of(message("Reserve successfully!")).await,
            "{\"message\":\"Reserve successfully!\"}\n"
        );
    }

    #[test]
    fn dates_and_numbers() {
        assert!(valid_date("2027-02-28"));
        assert!(!valid_date("2027-02-30"));
        assert!(!valid_date("2027-2-3"));
        assert!(!valid_date("abc"));
        assert_eq!(parse_f32("-122.41") as f32, -122.41f32);
        assert_eq!(parse_f32("x"), 0.0);
    }

    #[test]
    fn query_takes_the_first_value_and_decodes() {
        let req = Request::builder()
            .uri("/hotels?inDate=2027-01-01&name=a%20b&inDate=x")
            .body(Body::empty())
            .unwrap();
        let q = Query::of(&req);
        assert_eq!(q.get("inDate"), "2027-01-01");
        assert_eq!(q.get("name"), "a b");
        assert_eq!(q.get("missing"), "");
    }
}
