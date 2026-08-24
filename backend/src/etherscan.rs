use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        AssetSpec, Attribution, Chain, ChainAddress, ChainFamily, TokenAmount, TraceWindow,
        Transfer, TransferId,
    },
    provider::{ProviderError, TransferBatch, TransferProvider},
};

const PAGE_SIZE: usize = 1_000;
const BLOCK_RANGE_CACHE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EtherscanAction {
    Normal,
    Internal,
    Token,
}

impl EtherscanAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "txlist",
            Self::Internal => "txlistinternal",
            Self::Token => "tokentx",
        }
    }
}

#[derive(Clone, Copy)]
struct BlockRangeCache {
    key: (Chain, i64, i64),
    range: (u64, u64),
    cached_at: Instant,
}

#[derive(Clone)]
pub struct EtherscanClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    max_pages: usize,
    block_cache: Arc<Mutex<Option<BlockRangeCache>>>,
}

impl EtherscanClient {
    pub fn new(
        base_url: impl Into<String>,
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
            base_url: base_url.into(),
            api_key,
            max_pages,
            block_cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn block_by_time(
        &self,
        chain_id: u64,
        timestamp_ms: i64,
        closest: &str,
    ) -> Result<u64, ProviderError> {
        let query = [
            ("chainid", chain_id.to_string()),
            ("module", "block".to_owned()),
            ("action", "getblocknobytime".to_owned()),
            (
                "timestamp",
                timestamp_ms.div_euclid(1_000).max(0).to_string(),
            ),
            ("closest", closest.to_owned()),
            ("apikey", self.api_key.clone()),
        ];
        let response = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(|error| {
                ProviderError::Request(if error.is_timeout() {
                    "Etherscan block lookup timed out".to_owned()
                } else {
                    "Etherscan block lookup transport error".to_owned()
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
        let envelope = response.json::<EtherscanEnvelope>().await.map_err(|_| {
            ProviderError::InvalidResponse("Etherscan returned invalid JSON".to_owned())
        })?;
        let result = envelope.result.as_str().unwrap_or_default();
        if result.to_ascii_lowercase().contains("rate limit") {
            return Err(ProviderError::RateLimited);
        }
        result.parse::<u64>().map_err(|_| {
            ProviderError::InvalidResponse(format!("{}: invalid block result", envelope.message))
        })
    }

    async fn block_range(
        &self,
        address: &ChainAddress,
        window: TraceWindow,
    ) -> Result<(u64, u64), ProviderError> {
        let key = (address.chain(), window.min_timestamp, window.max_timestamp);
        let cached = *self.block_cache.lock().expect("block cache poisoned");
        if let Some(cached) = cached {
            if cached.key == key && cached.cached_at.elapsed() <= BLOCK_RANGE_CACHE_TTL {
                return Ok(cached.range);
            }
        }
        let chain_id = address
            .chain()
            .evm_chain_id()
            .ok_or(ProviderError::UnsupportedChain)?;
        let start = self
            .block_by_time(chain_id, window.min_timestamp, "after")
            .await?;
        let end = self
            .block_by_time(chain_id, window.max_timestamp, "before")
            .await?;
        *self.block_cache.lock().expect("block cache poisoned") = Some(BlockRangeCache {
            key,
            range: (start, end),
            cached_at: Instant::now(),
        });
        Ok((start, end))
    }

    async fn fetch_page(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        action: EtherscanAction,
        start_block: u64,
        end_block: u64,
        page: usize,
    ) -> Result<Vec<EtherscanRow>, ProviderError> {
        let chain_id = address
            .chain()
            .evm_chain_id()
            .ok_or(ProviderError::UnsupportedChain)?;
        let mut query = vec![
            ("chainid", chain_id.to_string()),
            ("module", "account".to_owned()),
            ("action", action.as_str().to_owned()),
            ("address", address.as_str().to_owned()),
            ("startblock", start_block.to_string()),
            ("endblock", end_block.to_string()),
            ("page", page.to_string()),
            ("offset", PAGE_SIZE.to_string()),
            ("sort", "asc".to_owned()),
            ("apikey", self.api_key.clone()),
        ];
        if let Some(contract) = asset.contract_address() {
            query.push(("contractaddress", contract.to_owned()));
        }
        let response = self
            .client
            .get(&self.base_url)
            .query(&query)
            .send()
            .await
            .map_err(|error| {
                ProviderError::Request(if error.is_timeout() {
                    "Etherscan request timed out".to_owned()
                } else {
                    "Etherscan transport error".to_owned()
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
        let envelope = response.json::<EtherscanEnvelope>().await.map_err(|_| {
            ProviderError::InvalidResponse("Etherscan returned invalid JSON".to_owned())
        })?;
        match envelope.result {
            serde_json::Value::Array(rows) => rows
                .into_iter()
                .map(|row| {
                    serde_json::from_value(row)
                        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))
                })
                .collect(),
            serde_json::Value::String(message)
                if message.to_ascii_lowercase().contains("no transactions") =>
            {
                Ok(Vec::new())
            }
            serde_json::Value::String(message)
                if message.to_ascii_lowercase().contains("rate limit") =>
            {
                Err(ProviderError::RateLimited)
            }
            other => Err(ProviderError::InvalidResponse(format!(
                "{}: unexpected result {other}",
                envelope.message
            ))),
        }
    }

    async fn fetch_pages(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        action: EtherscanAction,
        start_block: u64,
        end_block: u64,
        page_limit: usize,
    ) -> Result<(Vec<EtherscanRow>, bool), ProviderError> {
        let mut all_rows = Vec::new();
        for page in 1..=page_limit {
            let rows = self
                .fetch_page(address, asset, action, start_block, end_block, page)
                .await?;
            let is_last = rows.len() < PAGE_SIZE;
            all_rows.extend(rows);
            if is_last {
                return Ok((all_rows, false));
            }
        }
        Ok((all_rows, true))
    }

    fn map_row(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        action: EtherscanAction,
        row: EtherscanRow,
    ) -> Result<Option<Transfer>, ProviderError> {
        if error_flag_is_set(row.is_error.as_deref())
            || row.tx_receipt_status.as_deref() == Some("0")
            || (action == EtherscanAction::Internal
                && row
                    .err_code
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty() && value.trim() != "0"))
        {
            return Ok(None);
        }
        if matches!(asset, AssetSpec::Native) && row.value.chars().all(|character| character == '0')
        {
            return Ok(None);
        }
        if row.to.trim().is_empty() {
            return Ok(None);
        }
        let chain = address.chain();
        let from = ChainAddress::parse(chain, &row.from)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let to = ChainAddress::parse(chain, &row.to)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let decimals = row
            .token_decimal
            .as_deref()
            .and_then(|value| value.parse::<u8>().ok())
            .unwrap_or_else(|| asset.decimals(chain));
        let amount = TokenAmount::parse(&row.value, decimals)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let block_timestamp = row
            .time_stamp
            .parse::<i64>()
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?
            .saturating_mul(1_000);
        let event_index = (action == EtherscanAction::Token)
            .then(|| row.log_index.as_deref().and_then(parse_index))
            .flatten();
        let event_key = match action {
            EtherscanAction::Normal => "normal".to_owned(),
            EtherscanAction::Internal => row
                .trace_id
                .as_deref()
                .filter(|trace_id| !trace_id.trim().is_empty())
                .map(|trace_id| format!("internal-trace-{trace_id}"))
                .unwrap_or_else(|| format!("internal-synthetic-{}", synthetic_event_key(&row))),
            EtherscanAction::Token => event_index
                .map(|index| format!("log-{index}"))
                .unwrap_or_else(|| format!("token-synthetic-{}", synthetic_event_key(&row))),
        };
        Ok(Some(Transfer {
            id: TransferId {
                tx_id: row.hash,
                event_key,
            },
            chain,
            network: chain.network_name().to_owned(),
            asset_symbol: asset.symbol(chain).to_owned(),
            attribution: Attribution::Exact,
            from,
            to,
            amount,
            event_index,
            block_timestamp,
            confirmed: true,
        }))
    }
}

#[async_trait]
impl TransferProvider for EtherscanClient {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        if address.chain().family() != ChainFamily::Evm {
            return Err(ProviderError::UnsupportedChain);
        }
        let page_limit = max_pages.clamp(1, self.max_pages.max(1));
        let (start_block, end_block) = self.block_range(address, window).await?;
        if start_block > end_block {
            return Ok(TransferBatch {
                transfers: Vec::new(),
                truncated: false,
            });
        }
        let mut transfers = Vec::new();
        let mut synthetic_occurrences = HashMap::<String, usize>::new();
        let mut truncated = false;
        let actions: &[EtherscanAction] = if matches!(asset, AssetSpec::Native) {
            &[EtherscanAction::Normal, EtherscanAction::Internal]
        } else {
            &[EtherscanAction::Token]
        };
        for &action in actions {
            let (rows, action_truncated) = self
                .fetch_pages(address, asset, action, start_block, end_block, page_limit)
                .await?;
            truncated |= action_truncated;
            for row in rows {
                let Some(mut transfer) = self.map_row(address, asset, action, row)? else {
                    continue;
                };
                if transfer.id.event_key.contains("-synthetic-") {
                    let base_key = transfer.id.event_key.clone();
                    let occurrence = synthetic_occurrences.entry(base_key.clone()).or_insert(0);
                    let sequence = *occurrence;
                    *occurrence += 1;
                    transfer.id.event_key = format!("{base_key}-occurrence-{sequence}");
                }
                if transfer.block_timestamp >= window.min_timestamp
                    && transfer.block_timestamp <= window.max_timestamp
                {
                    transfers.push(transfer);
                }
            }
        }
        transfers.sort_by(|left, right| {
            (
                left.block_timestamp,
                left.id.tx_id.as_str(),
                left.id.event_key.as_str(),
            )
                .cmp(&(
                    right.block_timestamp,
                    right.id.tx_id.as_str(),
                    right.id.event_key.as_str(),
                ))
        });
        Ok(TransferBatch {
            transfers,
            truncated,
        })
    }
}

fn error_flag_is_set(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty() && value.trim() != "0")
}

fn parse_index(value: &str) -> Option<u32> {
    value
        .strip_prefix("0x")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| value.parse::<u32>().ok())
}

fn synthetic_event_key(row: &EtherscanRow) -> String {
    let mut hash = Sha256::new();
    for value in [
        row.hash.as_str(),
        row.from.as_str(),
        row.to.as_str(),
        row.value.as_str(),
        row.time_stamp.as_str(),
        row.transaction_index.as_deref().unwrap_or(""),
        row.contract_address.as_deref().unwrap_or(""),
        row.token_decimal.as_deref().unwrap_or(""),
        row.trace_id.as_deref().unwrap_or(""),
        row.err_code.as_deref().unwrap_or(""),
    ] {
        hash.update(value.as_bytes());
        hash.update([0]);
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Deserialize)]
struct EtherscanEnvelope {
    #[allow(dead_code)]
    status: String,
    message: String,
    result: serde_json::Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EtherscanRow {
    hash: String,
    from: String,
    to: String,
    value: String,
    time_stamp: String,
    #[serde(default)]
    is_error: Option<String>,
    #[serde(default)]
    #[serde(rename = "txreceipt_status")]
    tx_receipt_status: Option<String>,
    #[serde(default)]
    transaction_index: Option<String>,
    #[serde(default)]
    log_index: Option<String>,
    #[serde(default)]
    token_decimal: Option<String>,
    #[serde(default)]
    contract_address: Option<String>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    err_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{body::Body, http::Request, routing::any, Json, Router};

    use super::*;
    use crate::domain::Chain;

    fn client() -> EtherscanClient {
        EtherscanClient::new("https://api.etherscan.io/v2/api", "test-key", 10).unwrap()
    }

    #[test]
    fn maps_native_and_token_rows_without_losing_event_indexes() {
        let address = ChainAddress::parse(
            Chain::Ethereum,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
        )
        .unwrap();
        let native: EtherscanRow = serde_json::from_value(serde_json::json!({
            "hash": "0xabc", "from": address.as_str(),
            "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
            "value": "1500000000000000000", "timeStamp": "100", "isError": "0",
            "txreceipt_status": "1", "transactionIndex": "3"
        }))
        .unwrap();
        let mapped = client()
            .map_row(
                &address,
                &AssetSpec::native(),
                EtherscanAction::Normal,
                native,
            )
            .unwrap()
            .unwrap();
        assert_eq!(mapped.amount.display(), "1.5");
        assert_eq!(mapped.asset_symbol, "ETH");
        assert_eq!(mapped.id.event_key, "normal");

        let token: EtherscanRow = serde_json::from_value(serde_json::json!({
            "hash": "0xdef", "from": address.as_str(),
            "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
            "value": "2500000", "timeStamp": "101", "transactionIndex": "4",
            "tokenDecimal": "6", "contractAddress": "0x23954cfab0d22e650746cd2cdbf56eb326123325"
        }))
        .unwrap();
        let asset = AssetSpec::token("USDT", "0x23954cfab0d22e650746cd2cdbf56eb326123325", 6);
        let mapped = client()
            .map_row(&address, &asset, EtherscanAction::Token, token)
            .unwrap()
            .unwrap();
        assert_eq!(mapped.amount.display(), "2.5");
        assert_eq!(mapped.event_index, None);
        assert!(mapped.id.event_key.starts_with("token-synthetic-"));
    }

    #[tokio::test]
    async fn native_contract_merges_normal_and_internal_and_filters_invalid_internal_rows() {
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let request_log = Arc::clone(&requests);
        let app = Router::new().route(
            "/",
            any(move |request: Request<Body>| {
                let request_log = Arc::clone(&request_log);
                async move {
                    let uri = request.uri().to_string();
                    request_log.lock().unwrap().push(uri.clone());
                    let result = if uri.contains("action=getblocknobytime") {
                        if uri.contains("closest=after") {
                            serde_json::json!("10")
                        } else {
                            serde_json::json!("20")
                        }
                    } else if uri.contains("action=txlistinternal") {
                        serde_json::json!([
                            {
                                "hash": "0xinternal", "from": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
                                "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
                                "value": "500000000000000000", "timeStamp": "101",
                                "traceId": "call_0_1", "isError": "0", "errCode": ""
                            },
                            {
                                "hash": "0xfailed", "from": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
                                "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
                                "value": "700000000000000000", "timeStamp": "101",
                                "traceId": "call_0_2", "isError": "1", "errCode": "execution reverted"
                            },
                            {
                                "hash": "0xzero", "from": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
                                "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
                                "value": "0", "timeStamp": "101", "traceId": "call_0_3", "isError": "0"
                            }
                        ])
                    } else {
                        serde_json::json!([{
                            "hash": "0xnormal", "from": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
                            "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
                            "value": "1000000000000000000", "timeStamp": "100",
                            "transactionIndex": "3", "isError": "0", "txreceipt_status": "1"
                        }])
                    };
                    Json(serde_json::json!({
                        "status": "1",
                        "message": "OK",
                        "result": result
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            EtherscanClient::new(format!("http://{server_address}/"), "test-key", 1).unwrap();
        let address = ChainAddress::parse(
            Chain::Ethereum,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
        )
        .unwrap();

        let batch = client
            .transfers(
                &address,
                &AssetSpec::native(),
                TraceWindow {
                    min_timestamp: 99_000,
                    max_timestamp: 102_000,
                },
                1,
            )
            .await
            .unwrap();
        server.abort();

        assert!(!batch.truncated);
        assert_eq!(batch.transfers.len(), 2);
        assert_eq!(batch.transfers[0].id.event_key, "normal");
        assert_eq!(batch.transfers[1].id.event_key, "internal-trace-call_0_1");
        assert_eq!(batch.transfers[1].amount.display(), "0.5");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        let normal_request = requests
            .iter()
            .find(|uri| uri.contains("action=txlist&"))
            .expect("normal transaction request missing");
        let internal_request = requests
            .iter()
            .find(|uri| uri.contains("action=txlistinternal"))
            .expect("internal transaction request missing");
        for uri in [normal_request, internal_request] {
            assert!(uri.contains("chainid=1"));
            assert!(uri.contains("startblock=10"));
            assert!(uri.contains("endblock=20"));
            assert!(uri.contains("page=1"));
            assert!(uri.contains("offset=1000"));
            assert!(uri.contains("sort=asc"));
            assert!(uri.contains("apikey=test-key"));
        }
    }

    #[tokio::test]
    async fn a_full_action_page_is_conservatively_marked_truncated() {
        let rows = (0..PAGE_SIZE)
            .map(|index| {
                serde_json::json!({
                    "hash": format!("0x{index:064x}"),
                    "from": "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
                    "to": "0x149389ff0a7824d3172cad6c596498f384c1798e",
                    "value": "1",
                    "timeStamp": "100",
                    "isError": "0",
                    "txreceipt_status": "1"
                })
            })
            .collect::<Vec<_>>();
        let payload = Arc::new(serde_json::json!({
            "status": "1",
            "message": "OK",
            "result": rows
        }));
        let app = Router::new().route(
            "/",
            any(move || {
                let payload = Arc::clone(&payload);
                async move { Json((*payload).clone()) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let server_address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client =
            EtherscanClient::new(format!("http://{server_address}/"), "test-key", 1).unwrap();
        let address = ChainAddress::parse(
            Chain::Ethereum,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624",
        )
        .unwrap();

        let (rows, truncated) = client
            .fetch_pages(
                &address,
                &AssetSpec::native(),
                EtherscanAction::Normal,
                10,
                20,
                1,
            )
            .await
            .unwrap();
        server.abort();

        assert_eq!(rows.len(), PAGE_SIZE);
        assert!(truncated);
    }
}
