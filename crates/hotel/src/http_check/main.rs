//! HTTP API smoke check: calls every HTTP route of the frontend and validates
//! status codes and JSON shapes against what DeathStarBench's Go frontend (and
//! the vibesys generator's checks) expect. Routes the Rust port does not
//! support (owner writes) must answer 501 rather than pretend.
//!
//!   hotel_http_check --http http://frontend:5000 [--wait-secs 300]

use std::time::{Duration, Instant};

use clap::Parser;
use hyper::{Body, Client, Method, Request, StatusCode};
use serde_json::Value;

#[derive(Parser, Debug)]
struct Args {
    /// Base URL of the frontend's HTTP API.
    #[arg(long)]
    http: String,
    /// Retry the first request for this many seconds (readiness wait).
    #[arg(long, default_value_t = 0)]
    wait_secs: u64,
}

struct Reply {
    status: StatusCode,
    body: String,
}

/// One request; a request the policy sheds (`/EarlyReturn`, 503/504) is retried
/// a few times, since predictive admission may reject while it is cold.
async fn call(method: Method, url: &str) -> Result<Reply, Box<dyn std::error::Error>> {
    let mut attempt = 0;
    loop {
        let reply = call_once(method.clone(), url).await?;
        let shed =
            matches!(reply.status.as_u16(), 503 | 504) && reply.body.starts_with("/EarlyReturn");
        attempt += 1;
        if !shed || attempt >= 10 {
            return Ok(reply);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn call_once(method: Method, url: &str) -> Result<Reply, Box<dyn std::error::Error>> {
    let client = Client::new();
    let req = Request::builder()
        .method(method)
        .uri(url)
        .body(Body::empty())?;
    let resp = tokio::time::timeout(Duration::from_secs(10), client.request(req)).await??;
    let status = resp.status();
    let bytes = hyper::body::to_bytes(resp.into_body()).await?;
    Ok(Reply {
        status,
        body: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// A strict GeoJSON FeatureCollection with at least `min` features.
fn check_geojson(body: &str, min: usize) -> Result<usize, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
    if keys(&v) != ["features", "type"] || v["type"] != "FeatureCollection" {
        return Err(format!("root is not a FeatureCollection: {body}"));
    }
    let features = v["features"].as_array().ok_or("features is not an array")?;
    if features.len() < min {
        return Err(format!("{} features, want at least {min}", features.len()));
    }
    for f in features {
        if keys(f) != ["geometry", "id", "properties", "type"] || f["type"] != "Feature" {
            return Err(format!("bad feature: {f}"));
        }
        if keys(&f["properties"]) != ["name", "phone_number"] {
            return Err(format!("bad properties: {f}"));
        }
        let g = &f["geometry"];
        let ok = keys(g) == ["coordinates", "type"]
            && g["type"] == "Point"
            && g["coordinates"].as_array().map(|c| c.len()) == Some(2)
            && g["coordinates"][0].is_f64()
            && g["coordinates"][1].is_f64();
        if !ok || !f["id"].is_string() {
            return Err(format!("bad geometry or id: {f}"));
        }
    }
    Ok(features.len())
}

fn check_message(body: &str, allowed: &[&str]) -> Result<String, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
    let m = v["message"].as_str().unwrap_or_default().to_string();
    if keys(&v) != ["message"] || !allowed.contains(&m.as_str()) {
        return Err(format!(
            "unexpected message body {body:?}, allowed {allowed:?}"
        ));
    }
    Ok(m)
}

fn check_rates(body: &str, hotel: &str) -> Result<usize, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
    if keys(&v) != ["hotelId", "ratePlans"] || v["hotelId"] != hotel {
        return Err(format!("bad rates root: {body}"));
    }
    let plans = v["ratePlans"]
        .as_array()
        .ok_or("ratePlans is not an array")?;
    for p in plans {
        if keys(p) != ["code", "inDate", "outDate", "roomType"] {
            return Err(format!("bad plan: {p}"));
        }
        let want = [
            "bookableRate",
            "code",
            "roomDescription",
            "totalRate",
            "totalRateInclusive",
        ];
        if keys(&p["roomType"]) != want {
            return Err(format!("bad roomType: {p}"));
        }
    }
    Ok(plans.len())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let base = args.http.trim_end_matches('/').to_string();
    let mut failures: Vec<String> = Vec::new();

    let search =
        format!("{base}/hotels?inDate=2015-04-09&outDate=2015-04-10&lat=37.7867&lon=-122.4112");
    let deadline = Instant::now() + Duration::from_secs(args.wait_secs);
    loop {
        match call(Method::GET, &search).await {
            Ok(r) if r.status == StatusCode::OK => break,
            other if Instant::now() < deadline => {
                println!(
                    "waiting for the HTTP frontend: {:?}",
                    other.map(|r| r.status)
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Ok(r) => return Err(format!("frontend not ready: {} {}", r.status, r.body).into()),
            Err(e) => return Err(format!("frontend not ready: {e}").into()),
        }
    }

    let mut expect = |name: &str,
                      got: Result<Reply, Box<dyn std::error::Error>>,
                      status: u16,
                      check: &dyn Fn(&str) -> Result<String, String>| {
        let line = match got {
            Err(e) => Err(format!("request failed: {e}")),
            Ok(r) if r.status.as_u16() != status => Err(format!(
                "status {} (want {status}): {}",
                r.status,
                r.body.trim()
            )),
            Ok(r) => check(&r.body),
        };
        match line {
            Ok(note) => println!("ok   {name}: {note}"),
            Err(e) => {
                println!("FAIL {name}: {e}");
                failures.push(name.to_string());
            }
        }
    };

    let ok_geo = |min: usize| move |b: &str| check_geojson(b, min).map(|n| format!("{n} hotels"));
    let any_msg = |allowed: &'static [&'static str]| move |b: &str| check_message(b, allowed);
    let text = |b: &str| Ok(b.trim().to_string());

    expect(
        "GET /hotels",
        call(Method::GET, &search).await,
        200,
        &ok_geo(1),
    );
    expect(
        "GET /hotels locale=en",
        call(Method::GET, &format!("{search}&locale=en")).await,
        200,
        &ok_geo(1),
    );
    expect(
        "GET /hotels (no dates)",
        call(Method::GET, &format!("{base}/hotels?lat=1&lon=2")).await,
        400,
        &text,
    );
    expect(
        "GET /hotels (no location)",
        call(
            Method::GET,
            &format!("{base}/hotels?inDate=2015-04-09&outDate=2015-04-10"),
        )
        .await,
        400,
        &text,
    );
    for require in ["dis", "rate", "price"] {
        expect(
            &format!("GET /recommendations require={require}"),
            call(
                Method::GET,
                &format!("{base}/recommendations?require={require}&lat=37.7867&lon=-122.4112"),
            )
            .await,
            200,
            &ok_geo(1),
        );
    }
    expect(
        "GET /recommendations (bad require)",
        call(
            Method::GET,
            &format!("{base}/recommendations?require=x&lat=1&lon=2"),
        )
        .await,
        400,
        &text,
    );
    expect(
        "GET /user",
        call(
            Method::GET,
            &format!("{base}/user?username=Username_1&password=1111111111"),
        )
        .await,
        200,
        &any_msg(&["Login successfully!"]),
    );
    expect(
        "GET /user (wrong password)",
        call(
            Method::GET,
            &format!("{base}/user?username=Username_1&password=wrong"),
        )
        .await,
        200,
        &any_msg(&["Failed. Please check your username and password. "]),
    );
    expect(
        "GET /user (no password)",
        call(Method::GET, &format!("{base}/user?username=Username_1")).await,
        400,
        &text,
    );
    let reserve = format!(
        "{base}/reservation?inDate=2015-04-09&outDate=2015-04-10&hotelId=1&customerName=smoke\
         &username=Username_1&password=1111111111&number=1"
    );
    expect(
        "GET /reservation",
        call(Method::GET, &reserve).await,
        200,
        &any_msg(&["Reserve successfully!", "Failed. Already reserved. "]),
    );
    expect(
        "GET /reservation (bad date)",
        call(Method::GET, &reserve.replace("2015-04-10", "2015-4-10")).await,
        400,
        &text,
    );
    expect(
        "GET /rates",
        call(Method::GET, &format!("{base}/rates?hotelId=1")).await,
        200,
        &|b| check_rates(b, "1").map(|n| format!("{n} rate plans")),
    );
    expect(
        "GET /rates (no hotelId)",
        call(Method::GET, &format!("{base}/rates")).await,
        400,
        &text,
    );
    for route in ["rate", "profile", "capacity", "hotel"] {
        expect(
            &format!("POST /admin/{route} (unsupported)"),
            call(Method::POST, &format!("{base}/admin/{route}")).await,
            501,
            &text,
        );
        expect(
            &format!("GET /admin/{route}"),
            call(Method::GET, &format!("{base}/admin/{route}")).await,
            405,
            &text,
        );
    }

    if failures.is_empty() {
        println!("HTTP CHECK PASS");
        Ok(())
    } else {
        println!("HTTP CHECK FAIL: {}", failures.join(", "));
        std::process::exit(1);
    }
}
