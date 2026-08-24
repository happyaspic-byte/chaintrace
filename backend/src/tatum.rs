use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

use async_trait::async_trait;
use num_bigint::BigUint;
use reqwest::StatusCode;

use crate::{
    domain::{
        AssetSpec, Attribution, Chain, ChainAddress, ChainFamily, TokenAmount, TraceWindow,
        Transfer, TransferId,
    },
    provider::{ProviderError, TransferBatch, TransferProvider},
};

const PAGE_SIZE: usize = 50;

#[derive(Clone)]
pub struct TatumUtxoClient {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    max_pages: usize,
}

impl TatumUtxoClient {
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
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            max_pages,
        })
    }

    async fn fetch_page(
        &self,
        address: &ChainAddress,
        offset: usize,
    ) -> Result<Vec<serde_json::Value>, ProviderError> {
        let chain = match address.chain() {
            Chain::Bitcoin => "bitcoin-mainnet",
            Chain::Dogecoin => "doge-mainnet",
            Chain::Litecoin => "litecoin-mainnet",
            _ => return Err(ProviderError::UnsupportedChain),
        };
        let url = format!(
            "{}/v4/data/blockchains/transaction/history/utxos",
            self.base_url
        );
        let response = self
            .client
            .get(url)
            .header("x-api-key", &self.api_key)
            .query(&[
                ("chain", chain.to_owned()),
                ("address", address.as_str().to_owned()),
                ("pageSize", PAGE_SIZE.to_string()),
                ("offset", offset.to_string()),
            ])
            .send()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
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
        let payload = response
            .json::<serde_json::Value>()
            .await
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let rows = payload
            .as_array()
            .or_else(|| payload.get("result").and_then(serde_json::Value::as_array))
            .or_else(|| payload.get("data").and_then(serde_json::Value::as_array))
            .ok_or_else(|| {
                ProviderError::InvalidResponse(
                    "Tatum response omitted a transaction array".to_owned(),
                )
            })?;
        Ok(rows.clone())
    }

    fn map_transaction(
        &self,
        queried: &ChainAddress,
        row: &serde_json::Value,
    ) -> Result<Vec<Transfer>, ProviderError> {
        let tx_id = string_field(row, &["hash", "txId", "txid"])?;
        let confirmed = row
            .get("blockNumber")
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
            })
            .is_some_and(|block_number| block_number >= 0);
        if !confirmed {
            return Ok(Vec::new());
        }
        let timestamp = integer_field(row, &["time", "timestamp", "blockTime", "ts"])?;
        let block_timestamp = if timestamp < 10_000_000_000 {
            timestamp.saturating_mul(1_000)
        } else {
            timestamp
        };
        let inputs = row
            .get("inputs")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("UTXO transaction omitted inputs".to_owned())
            })?;
        let outputs = row
            .get("outputs")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("UTXO transaction omitted outputs".to_owned())
            })?;

        let mut input_weights = BTreeMap::<ChainAddress, BigUint>::new();
        for input in inputs {
            let Some(address_value) = input
                .pointer("/coin/address")
                .or_else(|| input.pointer("/prevout/scriptpubkey_address"))
                .or_else(|| input.get("address"))
                .and_then(serde_json::Value::as_str)
            else {
                return Ok(Vec::new());
            };
            let input_address = match ChainAddress::parse(queried.chain(), address_value) {
                Ok(address) => address,
                Err(_) => return Ok(Vec::new()),
            };
            let Some(value) = input
                .pointer("/coin/value")
                .or_else(|| input.pointer("/prevout/value"))
                .or_else(|| input.get("value"))
                .and_then(value_as_string)
            else {
                return Ok(Vec::new());
            };
            let raw = raw_utxo_value(&value)?;
            *input_weights
                .entry(input_address)
                .or_insert_with(|| BigUint::from(0_u8)) += raw;
        }
        let total_input = input_weights
            .values()
            .cloned()
            .fold(BigUint::from(0_u8), |sum, value| sum + value);
        if total_input == BigUint::from(0_u8) {
            return Ok(Vec::new());
        }
        let queried_is_input = input_weights.contains_key(queried);
        let queried_input_index = input_weights
            .keys()
            .position(|input_address| input_address == queried);
        let mut seen = HashSet::<String>::new();
        let mut transfers = Vec::new();

        for (output_index, output) in outputs.iter().enumerate() {
            let Some(output_address_value) = output
                .get("address")
                .or_else(|| output.get("scriptpubkey_address"))
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Ok(output_address) = ChainAddress::parse(queried.chain(), output_address_value)
            else {
                continue;
            };
            let value = output
                .get("value")
                .and_then(value_as_string)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("UTXO output omitted value".to_owned())
                })?;
            let output_raw = raw_utxo_value(&value)?;
            let allocations = proportional_allocations(&output_raw, &input_weights, &total_input);

            if queried_is_input && output_address != *queried {
                let event_key = format!(
                    "vin-{}-vout-{output_index}",
                    queried_input_index.expect("queried input index missing"),
                );
                if seen.insert(event_key.clone()) {
                    let allocated = allocations
                        .get(queried_input_index.expect("queried input index missing"))
                        .cloned()
                        .expect("queried allocation missing");
                    if allocated == BigUint::from(0_u8) {
                        continue;
                    }
                    let amount = TokenAmount::parse(
                        &allocated.to_str_radix(10),
                        queried.chain().native_decimals(),
                    )
                    .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
                    transfers.push(self.transfer(
                        queried,
                        &output_address,
                        &tx_id,
                        (event_key, output_index as u32),
                        amount,
                        block_timestamp,
                    ));
                }
            } else if !queried_is_input && output_address == *queried {
                for (input_index, input_address) in input_weights.keys().enumerate() {
                    let event_key = format!("vin-{input_index}-vout-{output_index}");
                    if seen.insert(event_key.clone()) {
                        let allocated = allocations
                            .get(input_index)
                            .cloned()
                            .expect("input allocation missing");
                        if allocated == BigUint::from(0_u8) {
                            continue;
                        }
                        let amount = TokenAmount::parse(
                            &allocated.to_str_radix(10),
                            queried.chain().native_decimals(),
                        )
                        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
                        transfers.push(self.transfer(
                            input_address,
                            queried,
                            &tx_id,
                            (event_key, output_index as u32),
                            amount,
                            block_timestamp,
                        ));
                    }
                }
            }
        }
        Ok(transfers)
    }

    fn transfer(
        &self,
        from: &ChainAddress,
        to: &ChainAddress,
        tx_id: &str,
        event: (String, u32),
        amount: TokenAmount,
        block_timestamp: i64,
    ) -> Transfer {
        let chain = from.chain();
        Transfer {
            id: TransferId {
                tx_id: tx_id.to_owned(),
                event_key: event.0,
            },
            chain,
            network: chain.network_name().to_owned(),
            asset_symbol: chain.native_symbol().to_owned(),
            attribution: Attribution::ProportionalHeuristic,
            from: from.clone(),
            to: to.clone(),
            amount,
            event_index: Some(event.1),
            block_timestamp,
            confirmed: true,
        }
    }
}

#[async_trait]
impl TransferProvider for TatumUtxoClient {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        if address.chain().family() != ChainFamily::Utxo {
            return Err(ProviderError::UnsupportedChain);
        }
        if !matches!(asset, AssetSpec::Native) {
            return Err(ProviderError::UnsupportedAsset);
        }
        let page_limit = max_pages.clamp(1, self.max_pages.max(1));
        let mut transfers = Vec::new();
        for page in 0..page_limit {
            let rows = self.fetch_page(address, page * PAGE_SIZE).await?;
            let is_last = rows.len() < PAGE_SIZE;
            for row in rows {
                transfers.extend(self.map_transaction(address, &row)?.into_iter().filter(
                    |transfer| {
                        transfer.block_timestamp >= window.min_timestamp
                            && transfer.block_timestamp <= window.max_timestamp
                    },
                ));
            }
            if is_last {
                return Ok(TransferBatch {
                    transfers,
                    truncated: false,
                });
            }
        }
        Ok(TransferBatch {
            transfers,
            truncated: true,
        })
    }
}

fn string_field(row: &serde_json::Value, names: &[&str]) -> Result<String, ProviderError> {
    names
        .iter()
        .find_map(|name| row.get(*name).and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .ok_or_else(|| {
            ProviderError::InvalidResponse(format!("missing field: {}", names.join(" or ")))
        })
}

fn integer_field(row: &serde_json::Value, names: &[&str]) -> Result<i64, ProviderError> {
    names
        .iter()
        .find_map(|name| {
            row.get(*name).and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
            })
        })
        .ok_or_else(|| {
            ProviderError::InvalidResponse(format!("missing integer field: {}", names.join(" or ")))
        })
}

fn value_as_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.is_number().then(|| value.to_string()))
}

fn raw_utxo_value(value: &str) -> Result<BigUint, ProviderError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProviderError::InvalidResponse(
            "invalid raw UTXO amount".to_owned(),
        ));
    }
    BigUint::parse_bytes(value.as_bytes(), 10)
        .ok_or_else(|| ProviderError::InvalidResponse("invalid UTXO amount".to_owned()))
}

fn proportional_allocations(
    output: &BigUint,
    input_weights: &BTreeMap<ChainAddress, BigUint>,
    total_input: &BigUint,
) -> Vec<BigUint> {
    let mut allocations = Vec::with_capacity(input_weights.len());
    let mut remainders = Vec::with_capacity(input_weights.len());
    let mut allocated_total = BigUint::from(0_u8);

    for (index, weight) in input_weights.values().enumerate() {
        let numerator = output * weight;
        let quotient = &numerator / total_input;
        let remainder = numerator % total_input;
        allocated_total += &quotient;
        allocations.push(quotient);
        remainders.push((index, remainder));
    }

    remainders.sort_by(|(left_index, left), (right_index, right)| {
        right.cmp(left).then_with(|| left_index.cmp(right_index))
    });
    let one = BigUint::from(1_u8);
    let zero = BigUint::from(0_u8);
    let mut remaining = output - allocated_total;
    for (index, _) in remainders {
        if remaining == zero {
            break;
        }
        allocations[index] += &one;
        remaining -= &one;
    }
    debug_assert_eq!(remaining, zero);
    allocations
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{body::Body, http::Request, routing::any, Json, Router};

    use super::*;

    fn client() -> TatumUtxoClient {
        TatumUtxoClient::new("https://api.tatum.io", "test-key", 10).unwrap()
    }

    #[tokio::test]
    async fn sends_the_documented_dogecoin_request_contract() {
        let requests = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let request_log = Arc::clone(&requests);
        let app = Router::new().route(
            "/v4/data/blockchains/transaction/history/utxos",
            any(move |request: Request<Body>| {
                let request_log = Arc::clone(&request_log);
                async move {
                    let api_key = request
                        .headers()
                        .get("x-api-key")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    request_log
                        .lock()
                        .unwrap()
                        .push((request.uri().to_string(), api_key));
                    Json(serde_json::json!([{
                        "hash": "documented-dogecoin-response",
                        "blockNumber": 1,
                        "time": 100,
                        "inputs": [{
                            "coin": {
                                "address": "DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY",
                                "value": "100000000"
                            }
                        }],
                        "outputs": [{
                            "address": "DFJfM8QrSJAqYTGDt7GNc24bHpuYZmUseB",
                            "value": "25000000"
                        }]
                    }]))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = TatumUtxoClient::new(format!("http://{address}"), "test-key", 1).unwrap();
        let dogecoin =
            ChainAddress::parse(Chain::Dogecoin, "DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY").unwrap();
        let batch = client
            .transfers(
                &dogecoin,
                &AssetSpec::native(),
                TraceWindow {
                    min_timestamp: 0,
                    max_timestamp: i64::MAX,
                },
                1,
            )
            .await
            .unwrap();
        server.abort();

        assert_eq!(batch.transfers.len(), 1);
        assert_eq!(batch.transfers[0].amount.raw_string(), "25000000");
        assert_eq!(batch.transfers[0].block_timestamp, 100_000);
        assert!(!batch.truncated);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let (uri, api_key) = &requests[0];
        assert!(uri.starts_with("/v4/data/blockchains/transaction/history/utxos?"));
        assert!(uri.contains("chain=doge-mainnet"));
        assert!(uri.contains("address=DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY"));
        assert!(uri.contains("pageSize=50"));
        assert!(uri.contains("offset=0"));
        assert_eq!(api_key, "test-key");
    }

    #[test]
    fn maps_outgoing_utxo_outputs_and_preserves_satoshis() {
        let queried =
            ChainAddress::parse(Chain::Bitcoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC").unwrap();
        let row = serde_json::json!({
            "hash": "tx-1",
            "blockNumber": 1,
            "time": 100,
            "inputs": [{ "coin": { "address": queried.as_str(), "value": "100000000" } }],
            "outputs": [
                { "address": "163ApeUHNC3dYEbZneW1STQbp1dAvTk9JM", "value": "25000000" },
                { "address": queried.as_str(), "value": "74990000" }
            ]
        });
        let transfers = client().map_transaction(&queried, &row).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].amount.raw_string(), "25000000");
        assert_eq!(transfers[0].amount.display(), "0.25");
    }

    #[test]
    fn parses_provider_values_as_raw_satoshis() {
        assert_eq!(
            raw_utxo_value("100000001").unwrap().to_str_radix(10),
            "100000001"
        );
        assert_eq!(raw_utxo_value("0").unwrap().to_str_radix(10), "0");
        assert_eq!(raw_utxo_value("1").unwrap().to_str_radix(10), "1");
        assert!(raw_utxo_value("0.1").is_err());
        assert!(raw_utxo_value("1.2.3").is_err());
    }

    #[test]
    fn incoming_multi_input_amount_is_proportionally_allocated() {
        let queried =
            ChainAddress::parse(Chain::Bitcoin, "1CDASWk8fywQRRVsSVgQzLgW1NbdYAFNbe").unwrap();
        let row = serde_json::json!({
            "hash": "tx-2",
            "blockNumber": 2,
            "time": 100,
            "inputs": [
                { "coin": { "address": "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC", "value": "75000000" } },
                { "coin": { "address": "163ApeUHNC3dYEbZneW1STQbp1dAvTk9JM", "value": "25000000" } }
            ],
            "outputs": [{ "address": queried.as_str(), "value": "40000000" }]
        });
        let transfers = client().map_transaction(&queried, &row).unwrap();
        assert_eq!(transfers.len(), 2);
        let total = transfers
            .iter()
            .map(|transfer| transfer.amount.raw().clone())
            .fold(BigUint::from(0_u8), |sum, value| sum + value);
        assert_eq!(total.to_str_radix(10), "40000000");

        let first_input =
            ChainAddress::parse(Chain::Bitcoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC").unwrap();
        let outgoing = client().map_transaction(&first_input, &row).unwrap();
        let incoming_from_first = transfers
            .iter()
            .find(|transfer| transfer.from == first_input)
            .unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].id, incoming_from_first.id);
        assert_eq!(outgoing[0].amount.raw_string(), "30000000");
    }

    #[test]
    fn skips_unconfirmed_utxo_transactions() {
        let queried =
            ChainAddress::parse(Chain::Bitcoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC").unwrap();
        let row = serde_json::json!({
            "hash": "mempool-tx",
            "blockNumber": -1,
            "time": 100,
            "inputs": [{ "coin": { "address": queried.as_str(), "value": "100000000" } }],
            "outputs": [{ "address": "163ApeUHNC3dYEbZneW1STQbp1dAvTk9JM", "value": "25000000" }]
        });
        assert!(client().map_transaction(&queried, &row).unwrap().is_empty());
    }

    #[test]
    fn preserves_single_satoshi_with_largest_remainder_allocation() {
        let queried =
            ChainAddress::parse(Chain::Bitcoin, "1CDASWk8fywQRRVsSVgQzLgW1NbdYAFNbe").unwrap();
        let row = serde_json::json!({
            "hash": "one-satoshi",
            "blockNumber": 3,
            "time": 100,
            "inputs": [
                { "coin": { "address": "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC", "value": "1" } },
                { "coin": { "address": "163ApeUHNC3dYEbZneW1STQbp1dAvTk9JM", "value": "1" } }
            ],
            "outputs": [{ "address": queried.as_str(), "value": "1" }]
        });
        let transfers = client().map_transaction(&queried, &row).unwrap();
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0].amount.raw_string(), "1");
    }

    #[test]
    fn skips_transactions_with_unattributable_inputs() {
        let queried =
            ChainAddress::parse(Chain::Bitcoin, "1CDASWk8fywQRRVsSVgQzLgW1NbdYAFNbe").unwrap();
        let row = serde_json::json!({
            "hash": "missing-prevout",
            "blockNumber": 4,
            "time": 100,
            "inputs": [
                { "coin": { "address": "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC", "value": "1" } },
                {}
            ],
            "outputs": [{ "address": queried.as_str(), "value": "1" }]
        });
        assert!(client().map_transaction(&queried, &row).unwrap().is_empty());
    }
}
