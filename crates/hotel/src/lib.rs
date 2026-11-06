/// Generated protobuf/gRPC code for every hotel service. Binaries include the
/// same generated files; this copy lets tests inspect the generated types.
pub mod proto {
    pub mod frontend {
        tonic::include_proto!("frontend");
    }
    pub mod search {
        tonic::include_proto!("search");
    }
    pub mod geo {
        tonic::include_proto!("geo");
    }
    pub mod rate {
        tonic::include_proto!("rate");
    }
    pub mod profile {
        tonic::include_proto!("profile");
    }
    pub mod reservation {
        tonic::include_proto!("reservation");
    }
    pub mod user {
        tonic::include_proto!("user");
    }
    pub mod recommendation {
        tonic::include_proto!("recommendation");
    }
    pub mod review {
        tonic::include_proto!("review");
    }
}

use std::time::Duration;

use tonic::transport::{Channel, Endpoint};

/// Connect to a downstream hotel service.
///
/// With one replica the service is addressed by its DNS name (`ip`), which is
/// the Compose service name or the Kubernetes Service name; the connection is
/// retried until the service is up. With `replicas > 1` the client balances
/// over `ip-1 .. ip-N` (one hostname per replica, e.g. Compose container
/// names), as in the original benchmark.
pub async fn connect(ip: String, port: u16, replicas: u8) -> Channel {
    if replicas <= 1 {
        let endpoint = Endpoint::from_shared(format!("http://{ip}:{port}")).unwrap();
        loop {
            match endpoint.connect().await {
                Ok(channel) => return channel,
                Err(error) => {
                    log::warn!("failed to connect to {}: {error}; retrying", endpoint.uri());
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        }
    } else {
        let endpoints = (1..=replicas)
            .map(|i| Endpoint::from_shared(format!("http://{ip}-{i}:{port}")).unwrap());
        Channel::balance_list(endpoints)
    }
}
