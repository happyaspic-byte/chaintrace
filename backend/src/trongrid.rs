use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        AssetSpec, Attribution, Chain, ChainAddress, TokenAmount, TraceWindow, Transfer, TransferId,
    },
    provider::{ProviderError, TransferBatch, TransferProvider},
};

pub const TRON_USDT_CONTRACT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

#[derive(Clone)]
pub struct TronGridClient {
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    network: String,
    max_pages: usize,
}

impl TronGridClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: Option<String>,
        network: impl Into<String>,
        max_pages: usize,
    ) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .user_agent("chaintrace/0.1")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            network: network.into(),
            max_pages,
        })
    }

    async fn fetch_page(
        &self,
        address: &ChainAddress,
        token_contract: &str,
        window: TraceWindow,
        fingerprint: Option<&str>,
    ) -> Result<TronGridResponse, ProviderError> {
        let url = format!(
            "{}/v1/accounts/{}/transactions/trc20",
            self.base_url,
            address.as_str()
        );
        let mut query = vec![
            ("only_confirmed", "true".to_owned()),
            ("contract_address", token_contract.to_owned()),
            ("limit", "200".to_owned()),
            ("order_by", "block_timestamp,asc".to_owned()),
            ("min_timestamp", window.min_timestamp.to_string()),
            ("max_timestamp", window.max_timestamp.to_string()),
        ];
        if let Some(value) = fingerprint {
            query.push(("fingerprint", value.to_owned()));
        }

        let mut request = self.client.get(url).query(&query);
        if let Some(api_key) = &self.api_key {
            request = request.header("TRON-PRO-API-KEY", api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        match response.status() {
            StatusCode::TOO_MANY_REQUESTS => return Err(ProviderError::RateLimited),
            StatusCode::FORBIDDEN => return Err(ProviderError::Forbidden),
            status if !status.is_success() => {
                return Err(ProviderError::Request(format!("HTTP {status}")))
            }
            _ => {}
        }
        let decoded = response
            .json::<TronGridResponse>()
            .await
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        if decoded.success == Some(false) {
            return Err(ProviderError::InvalidResponse(
                decoded
                    .message
                    .unwrap_or_else(|| "upstream returned success=false".to_owned()),
            ));
        }
        if decoded.data.is_none() {
            return Err(ProviderError::InvalidResponse(
                "upstream response omitted data".to_owned(),
            ));
        }
        Ok(decoded)
    }

    fn map_item(
        &self,
        item: TronGridItem,
        asset_symbol: &str,
    ) -> Result<Option<Transfer>, ProviderError> {
        if item.kind != "Transfer" {
            return Ok(None);
        }
        let from = ChainAddress::parse(Chain::Tron, &item.from)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let to = ChainAddress::parse(Chain::Tron, &item.to)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let amount = TokenAmount::parse(&item.value, item.token_info.decimals)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let event_key = format!("synthetic-{}", synthetic_event_key(&item));
        Ok(Some(Transfer {
            id: TransferId {
                tx_id: item.transaction_id,
                event_key,
            },
            chain: Chain::Tron,
            network: self.network.clone(),
            asset_symbol: asset_symbol.to_owned(),
            attribution: Attribution::Exact,
            from,
            to,
            amount,
            event_index: None,
            block_timestamp: item.block_timestamp,
            confirmed: true,
        }))
    }
}

#[async_trait]
impl TransferProvider for TronGridClient {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        if address.chain() != Chain::Tron {
            return Err(ProviderError::UnsupportedChain);
        }
        let token_contract = asset
            .contract_address()
            .ok_or(ProviderError::UnsupportedAsset)?;
        let asset_symbol = asset.symbol(Chain::Tron).to_owned();
        let mut fingerprint = None::<String>;
        let mut seen_fingerprints = HashSet::<String>::new();
        let mut seen_transfers = HashSet::<TransferId>::new();
        let mut synthetic_occurrences = HashMap::<String, usize>::new();
        let mut output = Vec::<Transfer>::new();
        let page_limit = max_pages.clamp(1, self.max_pages.max(1));

        for page_number in 0..page_limit {
            let page = self
                .fetch_page(address, token_contract, window, fingerprint.as_deref())
                .await?;
            let data = page.data.ok_or_else(|| {
                ProviderError::InvalidResponse("upstream response omitted data".to_owned())
            })?;
            for item in data {
                let Some(mut transfer) = self.map_item(item, &asset_symbol)? else {
                    continue;
                };
                let base_key = transfer.id.event_key.clone();
                let occurrence = synthetic_occurrences.entry(base_key.clone()).or_insert(0);
                let sequence = *occurrence;
                *occurrence += 1;
                transfer.id.event_key = format!("{base_key}-occurrence-{sequence}");
                if seen_transfers.insert(transfer.id.clone()) {
                    output.push(transfer);
                }
            }

            let Some(next) = page.meta.fingerprint else {
                return Ok(TransferBatch {
                    transfers: output,
                    truncated: false,
                });
            };
            if !seen_fingerprints.insert(next.clone()) {
                return Err(ProviderError::InvalidResponse(
                    "repeated pagination fingerprint".to_owned(),
                ));
            }
            fingerprint = Some(next);
            if page_number + 1 == page_limit {
                return Ok(TransferBatch {
                    transfers: output,
                    truncated: true,
                });
            }
        }
        Ok(TransferBatch {
            transfers: output,
            truncated: false,
        })
    }
}

fn synthetic_event_key(item: &TronGridItem) -> String {
    let mut hash = Sha256::new();
    hash.update(item.transaction_id.as_bytes());
    hash.update([0]);
    hash.update(item.from.as_bytes());
    hash.update([0]);
    hash.update(item.to.as_bytes());
    hash.update([0]);
    hash.update(item.value.as_bytes());
    hash.update(item.block_timestamp.to_be_bytes());
    let digest = hash.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Deserialize)]
struct TronGridResponse {
    data: Option<Vec<TronGridItem>>,
    #[serde(default)]
    meta: TronGridMeta,
    success: Option<bool>,
    message: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct TronGridMeta {
    fingerprint: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct TronGridItem {
    transaction_id: String,
    token_info: TokenInfo,
    block_timestamp: i64,
    from: String,
    to: String,
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize)]
struct TokenInfo {
    decimals: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(suffix: u8) -> ChainAddress {
        let mut payload = [0_u8; 21];
        payload[0] = 0x41;
        payload[20] = suffix;
        ChainAddress::tron_from_payload(payload).unwrap()
    }

    #[test]
    fn maps_transfer_and_ignores_approval_records() {
        let from = address(1);
        let to = address(2);
        let client =
            TronGridClient::new("https://api.trongrid.io", None, "tron-mainnet", 10).unwrap();
        let transfer: TronGridItem = serde_json::from_value(serde_json::json!({
            "transaction_id": "abc123",
            "token_info": { "decimals": 6 },
            "block_timestamp": 1000,
            "from": from.as_str(),
            "to": to.as_str(),
            "type": "Transfer",
            "value": "1500000"
        }))
        .unwrap();
        let mapped = client.map_item(transfer, "USDT").unwrap().unwrap();
        assert_eq!(mapped.event_index, None);
        assert!(mapped.id.event_key.starts_with("synthetic-"));
        assert_eq!(mapped.amount.display(), "1.5");

        let approval: TronGridItem = serde_json::from_value(serde_json::json!({
            "transaction_id": "def456",
            "token_info": { "decimals": 6 },
            "block_timestamp": 1001,
            "from": from.as_str(),
            "to": to.as_str(),
            "type": "Approval",
            "value": "1"
        }))
        .unwrap();
        assert!(client.map_item(approval, "USDT").unwrap().is_none());
    }

    #[test]
    fn synthetic_index_is_stable_for_duplicate_page_entries() {
        let from = address(1);
        let to = address(2);
        let item = TronGridItem {
            transaction_id: "same-tx".to_owned(),
            token_info: TokenInfo { decimals: 6 },
            block_timestamp: 42,
            from: from.to_string(),
            to: to.to_string(),
            kind: "Transfer".to_owned(),
            value: "1000000".to_owned(),
        };
        let copy = item.clone();
        assert_eq!(synthetic_event_key(&item), synthetic_event_key(&copy));
        assert_eq!(synthetic_event_key(&item).len(), 64);
    }
}
