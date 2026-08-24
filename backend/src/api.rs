use std::{sync::Arc, time::Duration};

use axum::{
    extract::{Request, State},
    http::{header::AUTHORIZATION, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{
    domain::{
        AssetSpec, Chain, ChainAddress, Direction, TokenAmount, TraceLimits, TraceParams,
        TraceWindow,
    },
    provider::{ProviderError, TransferProvider},
    tracer::{TraceEngine, TraceError, TraceGraph},
};

#[derive(Clone)]
struct AppState {
    engine: TraceEngine,
    trace_timeout: Duration,
    api_token: Arc<str>,
}

pub fn router(provider: Arc<dyn TransferProvider>, api_token: impl Into<String>) -> Router {
    router_with_timeout(provider, Duration::from_secs(45), api_token.into())
}

fn router_with_timeout(
    provider: Arc<dyn TransferProvider>,
    trace_timeout: Duration,
    api_token: String,
) -> Router {
    assert!(
        api_token.len() >= 32 && api_token.trim() == api_token,
        "API token must contain at least 32 non-whitespace characters"
    );
    let state = AppState {
        engine: TraceEngine::new(provider),
        trace_timeout,
        api_token: Arc::from(api_token),
    };
    Router::new()
        .route("/health", get(health))
        .route(
            "/api/v1/trace",
            post(create_trace).route_layer(middleware::from_fn_with_state(
                state.clone(),
                require_authentication,
            )),
        )
        .layer(ConcurrencyLimitLayer::new(16))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "service": "chaintrace-api", "known_chains": Chain::ALL }))
}

async fn create_trace(
    State(state): State<AppState>,
    Json(request): Json<TraceRequest>,
) -> Result<Json<TraceGraph>, ApiError> {
    let root = ChainAddress::parse(request.chain, &request.root_address)
        .map_err(|error| ApiError::unprocessable(error.to_string()))?;
    let asset = request
        .asset
        .clone()
        .unwrap_or_else(|| default_asset(request.chain));
    validate_asset(request.chain, &asset)?;
    let minimum = TokenAmount::parse(&request.min_amount_raw, asset.decimals(request.chain))
        .map_err(|error| ApiError::unprocessable(error.to_string()))?;
    let limits = request.limits.as_ref().cloned().unwrap_or_default();
    validate_request(&request, &limits)?;
    let params = TraceParams {
        root,
        asset,
        direction: request.direction,
        max_hops: request.max_hops,
        minimum,
        window: TraceWindow {
            min_timestamp: request.min_timestamp,
            max_timestamp: request.max_timestamp,
        },
        limits: TraceLimits {
            max_nodes: limits.max_nodes,
            max_edges: limits.max_edges,
            max_pages_per_address: limits.max_pages_per_address,
        },
    };
    let graph = tokio::time::timeout(state.trace_timeout, state.engine.trace(params))
        .await
        .map_err(|_| ApiError::gateway_timeout("trace exceeded the server deadline"))?
        .map_err(ApiError::from_trace)?;
    Ok(Json(graph))
}

async fn require_authentication(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    authorize(request.headers(), &state.api_token)?;
    Ok(next.run(request).await)
}

fn authorize(headers: &HeaderMap, expected_token: &str) -> Result<(), ApiError> {
    let provided = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if provided.is_some_and(|token| constant_time_eq(token.as_bytes(), expected_token.as_bytes())) {
        Ok(())
    } else {
        Err(ApiError::unauthorized("missing or invalid bearer token"))
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn validate_asset(chain: Chain, asset: &AssetSpec) -> Result<(), ApiError> {
    let AssetSpec::Token {
        symbol,
        contract_address,
        decimals,
    } = asset
    else {
        return Ok(());
    };
    if matches!(
        chain.family(),
        crate::domain::ChainFamily::Utxo | crate::domain::ChainFamily::Xrpl
    ) {
        return Err(ApiError::unprocessable(
            "token assets are not supported for this chain family",
        ));
    }
    if symbol.trim().is_empty() || symbol.len() > 16 {
        return Err(ApiError::unprocessable(
            "token symbol must be between 1 and 16 characters",
        ));
    }
    if symbol.trim() != symbol || contract_address.trim() != contract_address {
        return Err(ApiError::unprocessable(
            "token symbol and contract address must not contain surrounding whitespace",
        ));
    }
    if *decimals > 30 {
        return Err(ApiError::unprocessable("token decimals must not exceed 30"));
    }
    ChainAddress::parse(chain, contract_address)
        .map_err(|_| ApiError::unprocessable("token contract or mint address is invalid"))?;
    Ok(())
}

fn default_asset(chain: Chain) -> AssetSpec {
    if chain == Chain::Tron {
        AssetSpec::token("USDT", crate::trongrid::TRON_USDT_CONTRACT, 6)
    } else {
        AssetSpec::native()
    }
}

fn validate_request(request: &TraceRequest, limits: &LimitsRequest) -> Result<(), ApiError> {
    if request.min_timestamp > request.max_timestamp {
        return Err(ApiError::unprocessable(
            "min_timestamp must not be after max_timestamp",
        ));
    }
    let span = request
        .max_timestamp
        .checked_sub(request.min_timestamp)
        .ok_or_else(|| ApiError::unprocessable("timestamp range overflow"))?;
    const MAX_WINDOW_MS: i64 = 90 * 24 * 60 * 60 * 1_000;
    if span > MAX_WINDOW_MS {
        return Err(ApiError::unprocessable(
            "trace window must not exceed 90 days",
        ));
    }
    if !(1..=250).contains(&limits.max_nodes) {
        return Err(ApiError::unprocessable(
            "max_nodes must be between 1 and 250",
        ));
    }
    if !(1..=1_000).contains(&limits.max_edges) {
        return Err(ApiError::unprocessable(
            "max_edges must be between 1 and 1000",
        ));
    }
    if !(1..=10).contains(&limits.max_pages_per_address) {
        return Err(ApiError::unprocessable(
            "max_pages_per_address must be between 1 and 10",
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct TraceRequest {
    #[serde(default = "default_chain")]
    chain: Chain,
    root_address: String,
    #[serde(default)]
    asset: Option<AssetSpec>,
    #[serde(default = "default_direction")]
    direction: Direction,
    #[serde(default = "default_hops")]
    max_hops: u8,
    #[serde(default = "default_minimum")]
    min_amount_raw: String,
    min_timestamp: i64,
    max_timestamp: i64,
    #[serde(default)]
    limits: Option<LimitsRequest>,
}

fn default_chain() -> Chain {
    Chain::Tron
}
fn default_direction() -> Direction {
    Direction::Both
}
fn default_hops() -> u8 {
    2
}
fn default_minimum() -> String {
    "1000000".to_owned()
}

#[derive(Clone, Debug, Deserialize)]
struct LimitsRequest {
    #[serde(default = "default_max_nodes")]
    max_nodes: usize,
    #[serde(default = "default_max_edges")]
    max_edges: usize,
    #[serde(default = "default_max_pages")]
    max_pages_per_address: usize,
}

impl Default for LimitsRequest {
    fn default() -> Self {
        Self {
            max_nodes: default_max_nodes(),
            max_edges: default_max_edges(),
            max_pages_per_address: default_max_pages(),
        }
    }
}

fn default_max_nodes() -> usize {
    250
}
fn default_max_edges() -> usize {
    1_000
}
fn default_max_pages() -> usize {
    10
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }

    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
        }
    }

    fn from_trace(error: TraceError) -> Self {
        let status = match &error {
            TraceError::InvalidHops
            | TraceError::InvalidWindow
            | TraceError::InvalidLimits
            | TraceError::InvalidMinimumDecimals => StatusCode::UNPROCESSABLE_ENTITY,
            TraceError::Provider(
                ProviderError::UnsupportedChain
                | ProviderError::UnsupportedAsset
                | ProviderError::AssetMetadataMismatch
                | ProviderError::WindowOutsideRetention { .. },
            ) => StatusCode::UNPROCESSABLE_ENTITY,
            TraceError::Provider(ProviderError::UnconfiguredChain) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            TraceError::Provider(_) => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }

    fn gateway_timeout(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            message: message.into(),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use async_trait::async_trait;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        domain::TraceWindow,
        provider::{ProviderError, TransferBatch},
    };

    const TEST_API_TOKEN: &str = "test-chaintrace-api-token-with-32-characters";

    struct EmptyProvider;
    #[async_trait]
    impl TransferProvider for EmptyProvider {
        async fn transfers(
            &self,
            _address: &ChainAddress,
            _asset: &AssetSpec,
            _window: TraceWindow,
            _max_pages: usize,
        ) -> Result<TransferBatch, ProviderError> {
            Ok(TransferBatch {
                transfers: Vec::new(),
                truncated: false,
            })
        }
    }

    struct SlowProvider;
    #[async_trait]
    impl TransferProvider for SlowProvider {
        async fn transfers(
            &self,
            _address: &ChainAddress,
            _asset: &AssetSpec,
            _window: TraceWindow,
            _max_pages: usize,
        ) -> Result<TransferBatch, ProviderError> {
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(TransferBatch {
                transfers: Vec::new(),
                truncated: false,
            })
        }
    }

    fn valid_address() -> String {
        let mut payload = [0_u8; 21];
        payload[0] = 0x41;
        payload[20] = 9;
        ChainAddress::tron_from_payload(payload)
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn protects_trace_endpoint_with_bearer_authentication() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let body = json!({
            "root_address": valid_address(),
            "max_hops": 1,
            "min_timestamp": 0,
            "max_timestamp": 100
        });
        let missing = app
            .clone()
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .body(Body::from("{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let invalid = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn rejects_oversized_address_and_amount_inputs() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let oversized_address = json!({
            "chain": "solana",
            "root_address": "1".repeat(129),
            "max_hops": 1,
            "min_timestamp": 0,
            "max_timestamp": 100
        });
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(oversized_address.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let oversized_amount = json!({
            "root_address": valid_address(),
            "min_amount_raw": "9".repeat(129),
            "max_hops": 1,
            "min_timestamp": 0,
            "max_timestamp": 100
        });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(oversized_amount.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn rejects_invalid_address_with_422() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let body = json!({ "root_address": "not-an-address", "max_hops": 2, "min_timestamp": 0, "max_timestamp": 100 });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn returns_deterministic_empty_graph() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let body = json!({ "root_address": valid_address(), "direction": "both", "max_hops": 2, "min_timestamp": 0, "max_timestamp": 100 });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let graph: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 0);
        assert_eq!(graph["chain"], "tron");
        assert_eq!(graph["asset_symbol"], "USDT");
    }

    #[tokio::test]
    async fn accepts_an_evm_chain_and_native_asset() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let body = json!({
            "chain": "ethereum",
            "root_address": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
            "asset": { "kind": "native" },
            "max_hops": 1,
            "min_timestamp": 0,
            "max_timestamp": 100
        });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let graph: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(graph["chain"], "ethereum");
        assert_eq!(graph["asset_symbol"], "ETH");
    }

    #[tokio::test]
    async fn rejects_unbounded_limits() {
        let app = router(Arc::new(EmptyProvider), TEST_API_TOKEN);
        let body = json!({
            "root_address": valid_address(),
            "max_hops": 2,
            "min_timestamp": 0,
            "max_timestamp": 100,
            "limits": { "max_nodes": 251, "max_edges": 1000, "max_pages_per_address": 10 }
        });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn times_out_slow_traces() {
        let app = router_with_timeout(
            Arc::new(SlowProvider),
            Duration::from_millis(1),
            TEST_API_TOKEN.to_owned(),
        );
        let body = json!({
            "root_address": valid_address(),
            "direction": "both",
            "max_hops": 2,
            "min_timestamp": 0,
            "max_timestamp": 100
        });
        let response = app
            .oneshot(
                Request::post("/api/v1/trace")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_API_TOKEN}"))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    }
}
