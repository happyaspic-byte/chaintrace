use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        AssetSpec, Attribution, Chain, ChainAddress, TokenAmount, TraceWindow, Transfer, TransferId,
    },
    provider::{ProviderError, TransferBatch, TransferProvider},
};

const PAGE_SIZE: usize = 100;
const NATIVE_SOL_MINT: &str = "So11111111111111111111111111111111111111111";
const HELIUS_RETENTION_DAYS: u16 = 365;
const HELIUS_RETENTION_MS: i64 = (HELIUS_RETENTION_DAYS as i64) * 24 * 60 * 60 * 1_000;

#[derive(Clone)]
pub struct HeliusClient {
    client: reqwest::Client,
    rpc_url: String,
    api_key: String,
    max_pages: usize,
}

impl HeliusClient {
    pub fn new(
        rpc_url: impl Into<String>,
        api_key: impl Into<String>,
        max_pages: usize,
    ) -> Result<Self, ProviderError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(ProviderError::Forbidden);
        }
        let client = reqwest::Client::builder()
            .user_agent("chaintrace/0.2")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            rpc_url: rpc_url.into(),
            api_key,
            max_pages,
        })
    }

    async fn fetch_page(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        pagination_token: Option<&serde_json::Value>,
    ) -> Result<(Vec<serde_json::Value>, Option<serde_json::Value>), ProviderError> {
        let mint = asset.contract_address().unwrap_or(NATIVE_SOL_MINT);
        let mut options = json!({
            "limit": PAGE_SIZE,
            "sortOrder": "asc",
            "commitment": "finalized",
            "solMode": "merged",
            "mint": mint,
            "filters": {
                "blockTime": {
                    "gte": window.min_timestamp.div_euclid(1_000).max(0),
                    "lte": window.max_timestamp.div_euclid(1_000).max(0)
                }
            }
        });
        if let Some(token) = pagination_token {
            options["paginationToken"] = token.clone();
        }
        let response = self
            .client
            .post(self.rpc_url.trim_end_matches('?'))
            .query(&[("api-key", &self.api_key)])
            .json(&json!({
                "jsonrpc": "2.0",
                "id": "chaintrace",
                "method": "getTransfersByAddress",
                "params": [address.as_str(), options]
            }))
            .send()
            .await
            .map_err(|error| {
                ProviderError::Request(if error.is_timeout() {
                    "Helius request timed out".to_owned()
                } else {
                    "Helius transport error".to_owned()
                })
            })?;
        match response.status() {
            StatusCode::TOO_MANY_REQUESTS => return Err(ProviderError::RateLimited),
            StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED => {
                return Err(ProviderError::Forbidden)
            }
            status if !status.is_success() => {
                return Err(ProviderError::Request(format!("HTTP {status}")))
            }
            _ => {}
        }
        let payload = response.json::<serde_json::Value>().await.map_err(|_| {
            ProviderError::InvalidResponse("Helius returned invalid JSON".to_owned())
        })?;
        if let Some(error) = payload.get("error") {
            let message = error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Helius JSON-RPC error");
            if message.to_ascii_lowercase().contains("rate") {
                return Err(ProviderError::RateLimited);
            }
            return Err(ProviderError::InvalidResponse(message.to_owned()));
        }
        let result = payload.get("result").ok_or_else(|| {
            ProviderError::InvalidResponse("Helius response omitted result".to_owned())
        })?;
        let rows = result
            .get("data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("Helius response omitted data".to_owned())
            })?
            .clone();
        Ok((
            rows,
            result
                .get("paginationToken")
                .filter(|value| !value.is_null())
                .cloned(),
        ))
    }

    fn map_row(
        &self,
        asset: &AssetSpec,
        row: &serde_json::Value,
    ) -> Result<Option<Transfer>, ProviderError> {
        let transfer_type = row
            .get("type")
            .or_else(|| row.get("transferType"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("transfer")
            .to_ascii_lowercase();
        if !matches!(transfer_type.as_str(), "transfer" | "wrap" | "unwrap") {
            return Ok(None);
        }
        let Some(from_value) = row
            .get("fromUserAccount")
            .or_else(|| row.get("from"))
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(None);
        };
        let Some(to_value) = row
            .get("toUserAccount")
            .or_else(|| row.get("to"))
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(None);
        };
        let from = ChainAddress::parse(Chain::Solana, from_value)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let to = ChainAddress::parse(Chain::Solana, to_value)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let amount_raw = row.get("amount").and_then(value_as_string).ok_or_else(|| {
            ProviderError::InvalidResponse("Helius transfer omitted amount".to_owned())
        })?;
        let decimals = row
            .get("decimals")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or_else(|| asset.decimals(Chain::Solana));
        let amount = TokenAmount::parse(&amount_raw, decimals)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let signature = row
            .get("signature")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("Helius transfer omitted signature".to_owned())
            })?;
        let timestamp = row
            .get("blockTime")
            .or_else(|| row.get("timestamp"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            })
            .ok_or_else(|| {
                ProviderError::InvalidResponse("Helius transfer omitted blockTime".to_owned())
            })?;
        let transaction_index = row
            .get("transactionIdx")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let instruction_index = row
            .get("instructionIdx")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let inner_instruction_index = row
            .get("innerInstructionIdx")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let event_key = match (
            transaction_index,
            instruction_index,
            inner_instruction_index,
        ) {
            (Some(transaction), Some(instruction), inner) => {
                format!(
                    "tx-{transaction}-ix-{instruction}-inner-{}",
                    inner.map_or_else(|| "none".to_owned(), |value| value.to_string())
                )
            }
            _ => format!("synthetic-{}", synthetic_event_key(row)),
        };
        Ok(Some(Transfer {
            id: TransferId {
                tx_id: signature.to_owned(),
                event_key,
            },
            chain: Chain::Solana,
            network: Chain::Solana.network_name().to_owned(),
            asset_symbol: asset.symbol(Chain::Solana).to_owned(),
            attribution: Attribution::ProviderNormalized,
            from,
            to,
            amount,
            event_index: instruction_index,
            block_timestamp: timestamp.saturating_mul(1_000),
            confirmed: true,
        }))
    }
}

#[async_trait]
impl TransferProvider for HeliusClient {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        if address.chain() != Chain::Solana {
            return Err(ProviderError::UnsupportedChain);
        }
        validate_retention(window, unix_timestamp_ms())?;
        let page_limit = max_pages.clamp(1, self.max_pages.max(1));
        let mut pagination_token = None;
        let mut transfers = Vec::new();
        for page in 0..page_limit {
            let (rows, next_token) = self
                .fetch_page(address, asset, window, pagination_token.as_ref())
                .await?;
            for row in rows {
                let Some(transfer) = self.map_row(asset, &row)? else {
                    continue;
                };
                if transfer.block_timestamp >= window.min_timestamp
                    && transfer.block_timestamp <= window.max_timestamp
                {
                    transfers.push(transfer);
                }
            }
            let Some(next_token) = next_token else {
                return Ok(TransferBatch {
                    transfers,
                    truncated: false,
                });
            };
            pagination_token = Some(next_token);
            if page + 1 == page_limit {
                return Ok(TransferBatch {
                    transfers,
                    truncated: true,
                });
            }
        }
        Ok(TransferBatch {
            transfers,
            truncated: false,
        })
    }
}

fn unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn validate_retention(window: TraceWindow, now_ms: i64) -> Result<(), ProviderError> {
    let oldest_supported_timestamp = now_ms.saturating_sub(HELIUS_RETENTION_MS);
    if window.min_timestamp < oldest_supported_timestamp {
        Err(ProviderError::WindowOutsideRetention {
            provider: "Helius",
            retention_days: HELIUS_RETENTION_DAYS,
        })
    } else {
        Ok(())
    }
}

fn value_as_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}

fn synthetic_event_key(row: &serde_json::Value) -> String {
    let mut hash = Sha256::new();
    hash.update(row.to_string().as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{body::Body, extract::Request, routing::any, Router};

    use super::*;

    fn client() -> HeliusClient {
        HeliusClient::new("https://mainnet.helius-rpc.com/", "test-key", 10).unwrap()
    }

    #[test]
    fn maps_native_sol_transfer_with_raw_precision() {
        let row = json!({
            "signature": "sig-1",
            "fromUserAccount": "B7eKLu5DFf5bafEKpZPyBFuoBmMZtuworcbqDSaoPPDP",
            "toUserAccount": "47ddKiMQ9esPLDZQJWVE4NUe2ua7A7PJZJpmdbTTcA9g",
            "mint": NATIVE_SOL_MINT,
            "amount": "1500000000",
            "decimals": 9,
            "blockTime": 100,
            "type": "transfer",
            "transactionIdx": 1,
            "instructionIdx": 2,
            "innerInstructionIdx": 0
        });
        let mapped = client()
            .map_row(&AssetSpec::native(), &row)
            .unwrap()
            .unwrap();
        assert_eq!(mapped.amount.display(), "1.5");
        assert_eq!(mapped.event_index, Some(2));
        assert_eq!(mapped.asset_symbol, "SOL");
    }

    #[test]
    fn enforces_the_documented_one_year_retention_boundary() {
        let now_ms = 2_000_000_000_000;
        assert!(validate_retention(
            TraceWindow {
                min_timestamp: now_ms - HELIUS_RETENTION_MS,
                max_timestamp: now_ms,
            },
            now_ms,
        )
        .is_ok());

        let error = validate_retention(
            TraceWindow {
                min_timestamp: now_ms - HELIUS_RETENTION_MS - 1,
                max_timestamp: now_ms,
            },
            now_ms,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ProviderError::WindowOutsideRetention {
                provider: "Helius",
                retention_days: HELIUS_RETENTION_DAYS,
            }
        ));
    }

    #[tokio::test]
    async fn rejects_expired_windows_before_making_an_http_request() {
        let request_count = Arc::new(AtomicUsize::new(0));
        let observed_count = Arc::clone(&request_count);
        let app = Router::new().route(
            "/",
            any(move |_request: Request<Body>| {
                let observed_count = Arc::clone(&observed_count);
                async move {
                    observed_count.fetch_add(1, Ordering::SeqCst);
                    "unexpected request"
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = HeliusClient::new(format!("http://{server_address}/"), "test-key", 1).unwrap();
        let address = ChainAddress::parse(
            Chain::Solana,
            "B7eKLu5DFf5bafEKpZPyBFuoBmMZtuworcbqDSaoPPDP",
        )
        .unwrap();
        let now_ms = unix_timestamp_ms();

        let error = client
            .transfers(
                &address,
                &AssetSpec::native(),
                TraceWindow {
                    min_timestamp: now_ms - HELIUS_RETENTION_MS - 60_000,
                    max_timestamp: now_ms,
                },
                1,
            )
            .await
            .unwrap_err();
        server.abort();

        assert!(matches!(
            error,
            ProviderError::WindowOutsideRetention {
                provider: "Helius",
                retention_days: HELIUS_RETENTION_DAYS,
            }
        ));
        assert_eq!(request_count.load(Ordering::SeqCst), 0);
    }
}
