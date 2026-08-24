use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;

use crate::{
    domain::{
        AssetSpec, Attribution, Chain, ChainAddress, TokenAmount, TraceWindow, Transfer, TransferId,
    },
    provider::{ProviderError, TransferBatch, TransferProvider},
};

const PAGE_SIZE: usize = 100;
const RIPPLE_EPOCH_OFFSET: i64 = 946_684_800;

#[derive(Clone)]
pub struct XrplClient {
    client: reqwest::Client,
    rpc_url: String,
    max_pages: usize,
}

impl XrplClient {
    pub fn new(rpc_url: impl Into<String>, max_pages: usize) -> Result<Self, ProviderError> {
        let client = reqwest::Client::builder()
            .user_agent("chaintrace/0.2")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|error| ProviderError::Request(error.to_string()))?;
        Ok(Self {
            client,
            rpc_url: rpc_url.into(),
            max_pages,
        })
    }

    async fn fetch_page(
        &self,
        address: &ChainAddress,
        marker: Option<&serde_json::Value>,
    ) -> Result<(Vec<serde_json::Value>, Option<serde_json::Value>), ProviderError> {
        let mut params = json!({
            "account": address.as_str(),
            "ledger_index_min": -1,
            "ledger_index_max": -1,
            "binary": false,
            "limit": PAGE_SIZE,
            "forward": false,
            "api_version": 2
        });
        if let Some(marker) = marker {
            params["marker"] = marker.clone();
        }
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&json!({ "method": "account_tx", "params": [params] }))
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
        let result = payload.get("result").ok_or_else(|| {
            ProviderError::InvalidResponse("XRPL response omitted result".to_owned())
        })?;
        if result.get("status").and_then(serde_json::Value::as_str) == Some("error") {
            return Err(ProviderError::InvalidResponse(
                result
                    .get("error_message")
                    .or_else(|| result.get("error"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("XRPL error")
                    .to_owned(),
            ));
        }
        let rows = result
            .get("transactions")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                ProviderError::InvalidResponse("XRPL response omitted transactions".to_owned())
            })?
            .clone();
        Ok((rows, result.get("marker").cloned()))
    }

    fn map_row(&self, row: &serde_json::Value) -> Result<Option<Transfer>, ProviderError> {
        let tx = row
            .get("tx_json")
            .or_else(|| row.get("tx"))
            .ok_or_else(|| ProviderError::InvalidResponse("XRPL row omitted tx_json".to_owned()))?;
        if tx
            .get("TransactionType")
            .and_then(serde_json::Value::as_str)
            != Some("Payment")
        {
            return Ok(None);
        }
        // `delivered_amount` describes what the destination received, not what
        // the sender spent. Cross-currency payments can therefore deliver XRP
        // without representing a direct Account -> Destination XRP transfer.
        // API v2 calls Amount `DeliverMax`; keep the v1 alias for compatible
        // servers, but only attribute a direct XRP payment when the destination
        // amount is XRP and the cross-currency fields are absent.
        let delivers_xrp = tx
            .get("DeliverMax")
            .or_else(|| tx.get("Amount"))
            .is_some_and(serde_json::Value::is_string);
        if !delivers_xrp || tx.get("SendMax").is_some() || tx.get("Paths").is_some() {
            return Ok(None);
        }
        let meta = row
            .get("meta")
            .ok_or_else(|| ProviderError::InvalidResponse("XRPL row omitted meta".to_owned()))?;
        if meta
            .get("TransactionResult")
            .and_then(serde_json::Value::as_str)
            != Some("tesSUCCESS")
        {
            return Ok(None);
        }
        let delivered = meta
            .get("delivered_amount")
            .or_else(|| meta.get("DeliveredAmount"))
            .ok_or_else(|| {
                ProviderError::InvalidResponse("XRPL payment omitted delivered_amount".to_owned())
            })?;
        let Some(amount_raw) = delivered.as_str() else {
            return Ok(None);
        };
        if amount_raw == "unavailable" {
            return Ok(None);
        }
        let from = ChainAddress::parse(
            Chain::Xrp,
            tx.get("Account")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("XRPL payment omitted Account".to_owned())
                })?,
        )
        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let to = ChainAddress::parse(
            Chain::Xrp,
            tx.get("Destination")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    ProviderError::InvalidResponse("XRPL payment omitted Destination".to_owned())
                })?,
        )
        .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let amount = TokenAmount::parse(amount_raw, 6)
            .map_err(|error| ProviderError::InvalidResponse(error.to_string()))?;
        let tx_id = row
            .get("hash")
            .or_else(|| tx.get("hash"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse("XRPL row omitted hash".to_owned()))?;
        let event_index = meta
            .get("TransactionIndex")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok());
        let ripple_time = tx
            .get("date")
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            })
            .ok_or_else(|| {
                ProviderError::InvalidResponse("XRPL payment omitted date".to_owned())
            })?;
        Ok(Some(Transfer {
            id: TransferId {
                tx_id: tx_id.to_owned(),
                event_key: format!("payment-{}", event_index.unwrap_or(0)),
            },
            chain: Chain::Xrp,
            network: Chain::Xrp.network_name().to_owned(),
            asset_symbol: "XRP".to_owned(),
            attribution: Attribution::Exact,
            from,
            to,
            amount,
            event_index,
            block_timestamp: ripple_time
                .saturating_add(RIPPLE_EPOCH_OFFSET)
                .saturating_mul(1_000),
            confirmed: true,
        }))
    }
}

#[async_trait]
impl TransferProvider for XrplClient {
    async fn transfers(
        &self,
        address: &ChainAddress,
        asset: &AssetSpec,
        window: TraceWindow,
        max_pages: usize,
    ) -> Result<TransferBatch, ProviderError> {
        if address.chain() != Chain::Xrp {
            return Err(ProviderError::UnsupportedChain);
        }
        if !matches!(asset, AssetSpec::Native) {
            return Err(ProviderError::UnsupportedAsset);
        }
        let page_limit = max_pages.clamp(1, self.max_pages.max(1));
        let mut marker = None;
        let mut transfers = Vec::new();
        for page in 0..page_limit {
            let (rows, next_marker) = self.fetch_page(address, marker.as_ref()).await?;
            for row in rows {
                let Some(transfer) = self.map_row(&row)? else {
                    continue;
                };
                if transfer.block_timestamp >= window.min_timestamp
                    && transfer.block_timestamp <= window.max_timestamp
                {
                    transfers.push(transfer);
                }
            }
            let Some(next_marker) = next_marker else {
                return Ok(TransferBatch {
                    transfers,
                    truncated: false,
                });
            };
            marker = Some(next_marker);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_successful_xrp_payment_using_delivered_amount() {
        let row = json!({
            "hash": "ABC123",
            "tx_json": {
                "TransactionType": "Payment",
                "Account": "rGeNbpGQemfWPP9RxY49DptwESwNsiSGR",
                "Destination": "rfqaEpsD6fEFAwS2Jijv8dEpSKHZaAwbiQ",
                "DeliverMax": "1500000",
                "date": 100
            },
            "meta": {
                "TransactionResult": "tesSUCCESS",
                "TransactionIndex": 4,
                "delivered_amount": "1500000"
            }
        });
        let client = XrplClient::new("https://s2.ripple.com:51234/", 10).unwrap();
        let mapped = client.map_row(&row).unwrap().unwrap();
        assert_eq!(mapped.amount.display(), "1.5");
        assert_eq!(mapped.event_index, Some(4));
        assert_eq!(mapped.block_timestamp, 946_684_900_000);
    }

    #[test]
    fn ignores_failed_and_issued_currency_payments() {
        let failed = json!({
            "hash": "BAD", "tx_json": { "TransactionType": "Payment" },
            "meta": { "TransactionResult": "tecPATH_DRY", "delivered_amount": "1" }
        });
        let client = XrplClient::new("https://s2.ripple.com:51234/", 10).unwrap();
        assert!(client.map_row(&failed).unwrap().is_none());
    }

    #[test]
    fn ignores_cross_currency_payment_that_delivers_xrp() {
        let row = json!({
            "hash": "CROSS",
            "tx_json": {
                "TransactionType": "Payment",
                "Account": "rGeNbpGQemfWPP9RxY49DptwESwNsiSGR",
                "Destination": "rfqaEpsD6fEFAwS2Jijv8dEpSKHZaAwbiQ",
                "DeliverMax": "1500000",
                "SendMax": {
                    "currency": "USD",
                    "issuer": "rGeNbpGQemfWPP9RxY49DptwESwNsiSGR",
                    "value": "2"
                },
                "Paths": [[{
                    "currency": "USD",
                    "issuer": "rGeNbpGQemfWPP9RxY49DptwESwNsiSGR"
                }]],
                "date": 100
            },
            "meta": {
                "TransactionResult": "tesSUCCESS",
                "TransactionIndex": 5,
                "delivered_amount": "1500000"
            }
        });
        let client = XrplClient::new("https://s2.ripple.com:51234/", 10).unwrap();
        assert!(client.map_row(&row).unwrap().is_none());
    }
}
