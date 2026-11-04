//! Policy-liveness probe.
//!
//! Proves that real RPCs go through Masa's hooks in every hotel service, with a
//! check that cannot pass in baseline mode:
//!
//! 1. Control: `--control` Search requests through the frontend with a generous
//!    SLO must all succeed (in both modes), so the probe is not just rejecting
//!    everything.
//! 2. Expired deadline: for each service, send one request whose Masa context
//!    (the `ctx` header written by `RootContext::attach`) has an end-to-end
//!    deadline already in the past. Masa's `abort_slo` deadline guard runs in
//!    the server hooks before the handler is polled and must answer
//!    `DEADLINE_EXCEEDED "/EarlyReturn?src=<that service>::..."`. With no-op
//!    hooks the header is ignored, the handler runs, and no `/EarlyReturn` can
//!    be produced: that string only exists in Masa's policy modules.
//!
//! `--expect masa` requires an `/EarlyReturn` from every target;
//! `--expect baseline` requires none. Exit status 0 means the expectation held.

use std::time::Duration;

use clap::Parser;
use masa::RootContext;
use tonic::transport::Channel;
use tonic::{Request, Status};

use hotel::proto::*;

#[derive(Parser, Debug)]
struct Args {
    /// `baseline` or `masa`.
    #[arg(long)]
    expect: String,
    /// Frontend address, e.g. http://hotel-frontend:8660.
    #[arg(long)]
    frontend: String,
    /// Extra services to probe directly, as `name=http://host:port`. Known
    /// names: search, geo, rate, profile, reservation, user, recommendation,
    /// review.
    #[arg(long = "target")]
    targets: Vec<String>,
    /// Number of control Search requests (generous SLO) that must succeed.
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

#[derive(Debug)]
enum Outcome {
    Ok,
    EarlyReturn(String),
    Error(#[allow(dead_code)] String),
}

impl Outcome {
    fn of<T>(r: Result<tonic::Response<T>, Status>) -> Self {
        match r {
            Ok(_) => Outcome::Ok,
            Err(s) if s.message().starts_with("/EarlyReturn") => {
                Outcome::EarlyReturn(s.message().to_string())
            }
            Err(s) => Outcome::Error(format!("{:?}: {}", s.code(), s.message())),
        }
    }
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

/// A Masa root context whose end-to-end deadline has already passed.
async fn expired<T>(api: &str, msg: T) -> Request<T> {
    let ctx = RootContext::from(masa::create_context(api, Duration::from_micros(1)));
    tokio::time::sleep(Duration::from_millis(2)).await;
    ctx.attach(Request::new(msg))
}

async fn channel(addr: &str, timeout: Duration) -> Result<Channel, Box<dyn std::error::Error>> {
    Ok(Channel::from_shared(addr.to_string())?
        .timeout(timeout)
        .connect_timeout(timeout)
        .connect()
        .await?)
}

async fn probe(
    name: &str,
    addr: &str,
    timeout: Duration,
) -> Result<Outcome, Box<dyn std::error::Error>> {
    let ch = channel(addr, timeout).await?;
    let outcome = match name {
        "frontend" => {
            let mut c = frontend::frontend_client::FrontendClient::new(ch);
            Outcome::of(
                c.handle_search(expired("Search", search_request()).await)
                    .await,
            )
        }
        "search" => {
            let mut c = search::search_client::SearchClient::new(ch);
            Outcome::of(
                c.handle_nearby(expired("Search", Default::default()).await)
                    .await,
            )
        }
        "geo" => {
            let mut c = geo::geo_client::GeoClient::new(ch);
            Outcome::of(
                c.get_nearby(expired("Search", Default::default()).await)
                    .await,
            )
        }
        "rate" => {
            let mut c = rate::rate_client::RateClient::new(ch);
            Outcome::of(
                c.get_rates(expired("Search", Default::default()).await)
                    .await,
            )
        }
        "profile" => {
            let mut c = profile::profile_client::ProfileClient::new(ch);
            Outcome::of(
                c.get_profiles(expired("Search", Default::default()).await)
                    .await,
            )
        }
        "reservation" => {
            let mut c = reservation::reservation_client::ReservationClient::new(ch);
            Outcome::of(
                c.check_availability(expired("Search", Default::default()).await)
                    .await,
            )
        }
        "user" => {
            let mut c = user::user_client::UserClient::new(ch);
            Outcome::of(
                c.check_user(expired("Reservation", Default::default()).await)
                    .await,
            )
        }
        "recommendation" => {
            let mut c = recommendation::recommendation_client::RecommendationClient::new(ch);
            Outcome::of(
                c.get_recommendations(expired("Recommendation", Default::default()).await)
                    .await,
            )
        }
        "review" => {
            let mut c = review::review_client::ReviewClient::new(ch);
            Outcome::of(
                c.get_reviews(expired("Review", Default::default()).await)
                    .await,
            )
        }
        other => return Err(format!("unknown target {other}").into()),
    };
    Ok(outcome)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let masa_mode = match args.expect.as_str() {
        "masa" => true,
        "baseline" => false,
        other => return Err(format!("--expect must be baseline or masa, got {other}").into()),
    };
    let timeout = Duration::from_millis(args.timeout_ms);
    let mut failures = Vec::new();

    // 0. Readiness: wait until the frontend answers a Search end to end.
    let deadline = std::time::Instant::now() + Duration::from_secs(args.wait_secs);
    let mut fe = loop {
        let attempt = async {
            let ch = channel(&args.frontend, timeout).await?;
            let mut fe = frontend::frontend_client::FrontendClient::new(ch);
            let ctx = RootContext::from(masa::create_context("Search", Duration::from_secs(5)));
            fe.handle_search(ctx.attach(Request::new(search_request())))
                .await?;
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

    // 1. Control: generous SLO, must succeed in both modes.
    let mut ok = 0;
    for _ in 0..args.control {
        let ctx = RootContext::from(masa::create_context("Search", Duration::from_secs(5)));
        let req = ctx.attach(Request::new(search_request()));
        match fe.handle_search(req).await {
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

    // 2. Expired deadlines, one per service.
    let mut targets = vec![("frontend".to_string(), args.frontend.clone())];
    for t in &args.targets {
        let (name, addr) = t
            .split_once('=')
            .ok_or_else(|| format!("--target must be name=addr, got {t}"))?;
        targets.push((name.to_string(), addr.to_string()));
    }
    let mut early_returns = 0;
    for (name, addr) in &targets {
        let outcome = probe(name, addr, timeout).await?;
        println!("expired-deadline {name:<15} -> {outcome:?}");
        match (&outcome, masa_mode) {
            (Outcome::EarlyReturn(msg), true) => {
                early_returns += 1;
                // The rejecting hop must be the service we called directly.
                if !msg.contains(&format!("src={name}.")) {
                    failures.push(format!("{name}: early return from another hop: {msg}"));
                }
            }
            (Outcome::EarlyReturn(msg), false) => {
                early_returns += 1;
                failures.push(format!(
                    "{name}: baseline produced a Masa early return: {msg}"
                ));
            }
            (_, true) => failures.push(format!("{name}: expected /EarlyReturn, got {outcome:?}")),
            (_, false) => {}
        }
    }
    println!(
        "policy-live: {early_returns}/{} services rejected an expired Masa deadline (expect {})",
        targets.len(),
        args.expect
    );

    if failures.is_empty() {
        println!("PROBE PASS ({})", args.expect);
        Ok(())
    } else {
        for f in &failures {
            println!("FAIL: {f}");
        }
        println!("PROBE FAIL ({})", args.expect);
        std::process::exit(1);
    }
}
