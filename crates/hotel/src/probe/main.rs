//! Stack probe: proves the whole stack is up and answering.
//!
//! 1. Readiness: wait until the frontend answers a Search end to end.
//! 2. Control: `--control` Search requests through the frontend must all
//!    succeed.
//! 3. Every service (the frontend and each `--target name=http://host:port`)
//!    must answer one ordinary request, sent directly to it with a generous
//!    SLO, with success.
//!
//! The probe is deliberately policy-agnostic: the no-op `baseline` and the
//! initial `policy` stack must both pass, and so should any policy that does
//! not reject ordinary traffic. Exit status 0 means every check passed.

use std::time::Duration;

use app_utils::root_context::RootContext;
use clap::Parser;
use tonic::transport::Channel;
use tonic::{Request, Status};

use hotel::proto::*;

#[derive(Parser, Debug)]
struct Args {
    /// Frontend address, e.g. http://hotel-frontend:8660.
    #[arg(long)]
    frontend: String,
    /// Extra services to probe directly, as `name=http://host:port`. Known
    /// names: search, geo, rate, profile, reservation, user, recommendation,
    /// review.
    #[arg(long = "target")]
    targets: Vec<String>,
    /// Number of control Search requests that must succeed.
    #[arg(long, default_value_t = 20)]
    control: usize,
    /// Before probing, retry a Search for up to this many seconds until the
    /// stack answers (readiness wait).
    #[arg(long, default_value_t = 0)]
    wait_secs: u64,
    /// Per-request timeout in milliseconds.
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,
}

fn search_request() -> frontend::SearchRequest {
    frontend::SearchRequest {
        lat: 37.7867,
        lon: -122.4112,
        in_date: "2015-04-09".into(),
        out_date: "2015-04-10".into(),
        locale: None,
    }
}

/// A request with a generous (5 s) SLO.
fn live<T>(api: &str, msg: T) -> Request<T> {
    RootContext::new(api, Duration::from_secs(5)).attach(Request::new(msg))
}

async fn channel(addr: &str, timeout: Duration) -> Result<Channel, Box<dyn std::error::Error>> {
    Ok(Channel::from_shared(addr.to_string())?
        .timeout(timeout)
        .connect_timeout(timeout)
        .connect()
        .await?)
}

/// One ordinary request to the named service.
async fn probe(
    name: &str,
    addr: &str,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let ch = channel(addr, timeout).await?;
    let ids = vec!["1".to_string()];
    let (in_date, out_date) = ("2015-04-09".to_string(), "2015-04-10".to_string());
    let r: Result<(), Status> = match name {
        "frontend" => {
            let mut c = frontend::frontend_client::FrontendClient::new(ch);
            c.handle_search(live("Search", search_request()))
                .await
                .map(|_| ())
        }
        "search" => {
            let mut c = search::search_client::SearchClient::new(ch);
            let req = search::NearbyRequest {
                lat: 37.7867,
                lon: -122.4112,
                in_date,
                out_date,
            };
            c.handle_nearby(live("Search", req)).await.map(|_| ())
        }
        "geo" => {
            let mut c = geo::geo_client::GeoClient::new(ch);
            let req = geo::NearbyRequest {
                lat: 37.7867,
                lon: -122.4112,
            };
            c.get_nearby(live("Search", req)).await.map(|_| ())
        }
        "rate" => {
            let mut c = rate::rate_client::RateClient::new(ch);
            let req = rate::RateRequest {
                hotel_ids: ids,
                in_date,
                out_date,
            };
            c.get_rates(live("Search", req)).await.map(|_| ())
        }
        "profile" => {
            let mut c = profile::profile_client::ProfileClient::new(ch);
            let req = profile::ProfileRequest {
                hotel_ids: ids,
                locale: "en".into(),
            };
            c.get_profiles(live("Search", req)).await.map(|_| ())
        }
        "reservation" => {
            let mut c = reservation::reservation_client::ReservationClient::new(ch);
            let req = reservation::ReservationRequest {
                customer_name: String::new(),
                hotel_ids: ids,
                in_date,
                out_date,
                room_number: 1,
            };
            c.check_availability(live("Search", req)).await.map(|_| ())
        }
        "user" => {
            let mut c = user::user_client::UserClient::new(ch);
            let req = user::UserRequest {
                username: "Username_1".into(),
                password: "Password_1".into(),
            };
            c.check_user(live("Reservation", req)).await.map(|_| ())
        }
        "recommendation" => {
            let mut c = recommendation::recommendation_client::RecommendationClient::new(ch);
            let req = recommendation::RecommendationRequest {
                require: "rate".into(),
                lat: 37.7867,
                lon: -122.4112,
            };
            c.get_recommendations(live("Recommendation", req))
                .await
                .map(|_| ())
        }
        "review" => {
            let mut c = review::review_client::ReviewClient::new(ch);
            let req = review::ReviewRequest {
                hotel_id: "1".into(),
            };
            c.get_reviews(live("Review", req)).await.map(|_| ())
        }
        other => return Err(format!("unknown target {other}").into()),
    };
    r.map_err(|s| format!("{:?}: {}", s.code(), s.message()).into())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let timeout = Duration::from_millis(args.timeout_ms);
    let mut failures = Vec::new();

    // 1. Readiness: wait until the frontend answers a Search end to end.
    let deadline = std::time::Instant::now() + Duration::from_secs(args.wait_secs);
    let mut fe = loop {
        let attempt = async {
            let ch = channel(&args.frontend, timeout).await?;
            let mut fe = frontend::frontend_client::FrontendClient::new(ch);
            fe.handle_search(live("Search", search_request())).await?;
            Ok::<_, Box<dyn std::error::Error>>(fe)
        };
        match attempt.await {
            Ok(fe) => break fe,
            Err(e) if std::time::Instant::now() < deadline => {
                println!("waiting for frontend: {e}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(format!("frontend not ready: {e}").into()),
        }
    };

    // 2. Control: Search requests through the frontend.
    let mut ok = 0;
    for _ in 0..args.control {
        match fe.handle_search(live("Search", search_request())).await {
            Ok(_) => ok += 1,
            Err(s) => println!("control: error {:?}: {}", s.code(), s.message()),
        }
    }
    println!(
        "control: {ok}/{} Search requests with a 5 s SLO succeeded",
        args.control
    );
    if ok != args.control {
        failures.push(format!("control: only {ok}/{} succeeded", args.control));
    }

    // 3. One ordinary request to every service.
    let mut targets = vec![("frontend".to_string(), args.frontend.clone())];
    for t in &args.targets {
        let (name, addr) = t
            .split_once('=')
            .ok_or_else(|| format!("--target must be name=addr, got {t}"))?;
        targets.push((name.to_string(), addr.to_string()));
    }
    for (name, addr) in &targets {
        match probe(name, addr, timeout).await {
            Ok(()) => println!("service {name:<15} -> ok"),
            Err(e) => {
                println!("service {name:<15} -> {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }

    if failures.is_empty() {
        println!("PROBE PASS");
        Ok(())
    } else {
        for f in &failures {
            println!("FAIL: {f}");
        }
        println!("PROBE FAIL");
        std::process::exit(1);
    }
}
