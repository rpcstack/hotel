use serde::{Deserialize, Serialize};
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlobalConfig {
    pub hotels: u32,
    pub cache_conns: u32,
    pub prob_cache_miss: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasicServiceConfig {
    pub ip: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "one")]
    pub replicas: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CachedServiceConfig {
    #[serde(flatten)]
    pub endpoint: BasicServiceConfig,
    pub mongodb_addr: String,
    pub redis_addr: String,
}

impl Deref for CachedServiceConfig {
    type Target = BasicServiceConfig;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl DerefMut for CachedServiceConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoint
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserServiceConfig {
    #[serde(flatten)]
    pub endpoint: BasicServiceConfig,
    pub mongodb_addr: String,
    pub users: u32,
    pub prob_check_user: u32,
}

impl Deref for UserServiceConfig {
    type Target = BasicServiceConfig;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl DerefMut for UserServiceConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoint
    }
}

/// Per-route SLOs (milliseconds) the HTTP frontend gives each request's root
/// context: the request's deadline is "now + SLO" when it enters the RPC stack.
/// Application configuration, deliberately outside the policy directory so a
/// policy cannot relax its own deadlines. Defaults are what
/// `config/gen_config.json` gives the gRPC load generator (200 ms per API).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpSlosMs {
    /// `GET /hotels`; root-context API name `Search`.
    #[serde(default = "default_slo_ms")]
    pub search: u64,
    /// `GET /reservation`; API name `Reservation`.
    #[serde(default = "default_slo_ms")]
    pub reservation: u64,
    /// `GET /recommendations`; API name `Recommendation`.
    #[serde(default = "default_slo_ms")]
    pub recommendation: u64,
    /// `GET /user`; API name `User`.
    #[serde(default = "default_slo_ms")]
    pub user: u64,
    /// `GET /rates`; API name `Rates`.
    #[serde(default = "default_slo_ms")]
    pub rates: u64,
}

impl Default for HttpSlosMs {
    fn default() -> Self {
        Self {
            search: default_slo_ms(),
            reservation: default_slo_ms(),
            recommendation: default_slo_ms(),
            user: default_slo_ms(),
            rates: default_slo_ms(),
        }
    }
}

/// The frontend: the gRPC endpoint (`port`) plus the HTTP/JSON endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrontendServiceConfig {
    #[serde(flatten)]
    pub endpoint: BasicServiceConfig,
    /// Port of the HTTP/JSON API (5000, like DeathStarBench's Go frontend).
    #[serde(default = "default_http_port")]
    pub http_port: u16,
    #[serde(default)]
    pub slo_ms: HttpSlosMs,
}

impl Deref for FrontendServiceConfig {
    type Target = BasicServiceConfig;

    fn deref(&self) -> &Self::Target {
        &self.endpoint
    }
}

impl DerefMut for FrontendServiceConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.endpoint
    }
}

pub type FrontendConfig = FrontendServiceConfig;
pub type GeoConfig = BasicServiceConfig;
pub type ProfileConfig = CachedServiceConfig;
pub type RateConfig = CachedServiceConfig;
pub type RecommendationConfig = BasicServiceConfig;
pub type ReservationConfig = CachedServiceConfig;
pub type ReviewConfig = CachedServiceConfig;
pub type SearchConfig = BasicServiceConfig;
pub type UserConfig = UserServiceConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotelConfig {
    pub global: GlobalConfig,
    pub frontend: FrontendConfig,
    pub geo: GeoConfig,
    pub profile: ProfileConfig,
    pub rate: RateConfig,
    pub recommendation: RecommendationConfig,
    pub reservation: ReservationConfig,
    pub review: ReviewConfig,
    pub search: SearchConfig,
    pub user: UserConfig,
}

fn default_http_port() -> u16 {
    5000
}

fn default_slo_ms() -> u64 {
    200
}

fn one() -> u8 {
    1
}

fn default_port() -> u16 {
    8000
}
