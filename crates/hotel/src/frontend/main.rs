#[path = "../config.rs"]
pub mod config;
mod http;
pub mod server;

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;

use structopt::StructOpt;
use tonic::transport::Server;

use app_utils::logging::init_logging;
use config::HotelConfig;
use server::hotel_tonic::frontend::frontend_server::FrontendServer;
use server::FrontendImpl;

#[derive(StructOpt, Debug, Clone)]
#[structopt(about = "Hotel Args")]
pub struct Args {
    #[structopt(short, long, required = true)]
    pub config: PathBuf,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    main_inner().await
}

async fn main_inner() -> Result<(), Box<dyn std::error::Error>> {
    init_logging();

    let args = Args::from_args();
    let cfg: HotelConfig = {
        let file = File::open(args.config).expect("Failed to open file");
        let reader = BufReader::new(file);
        serde_json::from_reader(reader)?
    };

    let frontend_addr = format!("{}:{}", "[::]", cfg.frontend.port)
        .parse()
        .expect("Failed to parse address");
    let http_addr = format!("{}:{}", "[::]", cfg.frontend.http_port)
        .parse()
        .expect("Failed to parse address");
    let slo = cfg.frontend.slo_ms.clone();
    let frontend_service = FrontendImpl::new(cfg).await;
    // One server instance (one set of policy server state) behind both the
    // gRPC port and the in-process client the HTTP API calls.
    let server = FrontendServer::new(frontend_service);
    let http_state = Arc::new(http::HttpFrontend::new(
        http::LocalClient::new(http::InProcess::new(server.clone())),
        slo,
    ));
    tokio::spawn(async move {
        log::info!("HTTP API listening on {}...", http_addr);
        if let Err(e) = http::serve(http_addr, http_state).await {
            log::error!("HTTP server failed: {e}");
            std::process::exit(1);
        }
    });
    log::info!("gRPC server listening on {}...", frontend_addr);
    Server::builder()
        .add_service(server)
        .serve(frontend_addr)
        .await?;

    Ok(())
}
