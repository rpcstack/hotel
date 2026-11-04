pub mod profile_layer;

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

use masa::transport::LoadBalancedChannel;

/// Connect to a downstream hotel service.
///
/// With one replica the service is addressed by its DNS name (`ip`), which is
/// the Compose service name or the Kubernetes Service name. With `replicas > 1`
/// Masa's client-side round robin dials `ip-1 .. ip-N` (one hostname per
/// replica, e.g. Compose container names), as in the original benchmark.
pub async fn connect(ip: String, port: u16, replicas: u8) -> LoadBalancedChannel {
    if replicas <= 1 {
        LoadBalancedChannel::new_from_service_name(ip, port, replicas).await
    } else {
        LoadBalancedChannel::new(ip, port, replicas).await
    }
}
