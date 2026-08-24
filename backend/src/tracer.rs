use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use serde::Serialize;

use crate::{
    domain::{ChainAddress, Direction, TraceParams, Transfer, TransferId},
    provider::{ProviderError, TransferProvider},
};

const MAX_PROVIDER_QUERIES: usize = 50;

#[derive(Clone)]
pub struct TraceEngine {
    provider: Arc<dyn TransferProvider>,
}

impl TraceEngine {
    pub fn new(provider: Arc<dyn TransferProvider>) -> Self {
        Self { provider }
    }

    pub async fn trace(&self, params: TraceParams) -> Result<TraceGraph, TraceError> {
        if !(1..=3).contains(&params.max_hops) {
            return Err(TraceError::InvalidHops);
        }
        if params.window.min_timestamp > params.window.max_timestamp {
            return Err(TraceError::InvalidWindow);
        }
        if !(1..=250).contains(&params.limits.max_nodes)
            || !(1..=1_000).contains(&params.limits.max_edges)
            || !(1..=10).contains(&params.limits.max_pages_per_address)
        {
            return Err(TraceError::InvalidLimits);
        }

        let root = params.root.clone();
        let expected_chain = root.chain();
        let expected_symbol = params.asset.symbol(expected_chain).to_owned();
        let expected_decimals = params.asset.decimals(expected_chain);
        if params.minimum.decimals() != expected_decimals {
            return Err(TraceError::InvalidMinimumDecimals);
        }
        let mut queue = VecDeque::from([(root.clone(), 0_u8)]);
        let mut expanded = HashSet::<ChainAddress>::new();
        let mut depths = HashMap::<ChainAddress, u8>::from([(root.clone(), 0)]);
        let mut edges = HashMap::<TransferId, Transfer>::new();
        let mut truncation_reasons = Vec::<String>::new();
        let mut provider_queries = 0_usize;

        'search: while let Some((address, depth)) = queue.pop_front() {
            if depth >= params.max_hops || !expanded.insert(address.clone()) {
                continue;
            }
            if provider_queries >= MAX_PROVIDER_QUERIES {
                truncation_reasons.push("max_provider_queries".to_owned());
                break;
            }
            provider_queries += 1;

            let batch = self
                .provider
                .transfers(
                    &address,
                    &params.asset,
                    params.window,
                    params.limits.max_pages_per_address,
                )
                .await?;
            if batch.truncated
                && !truncation_reasons
                    .iter()
                    .any(|reason| reason == "max_pages_per_address")
            {
                truncation_reasons.push("max_pages_per_address".to_owned());
            }
            let mut transfers = batch.transfers;
            transfers.sort_by(|left, right| {
                (left.block_timestamp, &left.id.tx_id, &left.id.event_key).cmp(&(
                    right.block_timestamp,
                    &right.id.tx_id,
                    &right.id.event_key,
                ))
            });

            for transfer in transfers {
                if transfer.chain != expected_chain
                    || transfer.from.chain() != expected_chain
                    || transfer.to.chain() != expected_chain
                    || transfer.asset_symbol != expected_symbol
                {
                    return Err(ProviderError::InvalidResponse(
                        "provider returned a transfer outside the requested chain or asset"
                            .to_owned(),
                    )
                    .into());
                }
                if transfer.amount.decimals() != expected_decimals {
                    return Err(ProviderError::AssetMetadataMismatch.into());
                }
                if transfer.block_timestamp < params.window.min_timestamp
                    || transfer.block_timestamp > params.window.max_timestamp
                    || !transfer.confirmed
                    || transfer.amount.raw() < params.minimum.raw()
                    || !matches_direction(params.direction, &address, &transfer)
                {
                    continue;
                }
                if edges.contains_key(&transfer.id) {
                    continue;
                }
                if edges.len() >= params.limits.max_edges {
                    truncation_reasons.push("max_edges".to_owned());
                    break 'search;
                }

                let counterpart = if transfer.from == address {
                    transfer.to.clone()
                } else {
                    transfer.from.clone()
                };
                let candidate_depth = depth.saturating_add(1);
                if !depths.contains_key(&counterpart) && depths.len() >= params.limits.max_nodes {
                    truncation_reasons.push("max_nodes".to_owned());
                    break 'search;
                }
                edges.insert(transfer.id.clone(), transfer);

                let improves_depth = match depths.get(&counterpart) {
                    Some(existing) => candidate_depth < *existing,
                    None => true,
                };
                if improves_depth {
                    depths.insert(counterpart.clone(), candidate_depth);
                }
                if counterpart != address
                    && candidate_depth < params.max_hops
                    && improves_depth
                    && !expanded.contains(&counterpart)
                {
                    queue.push_back((counterpart, candidate_depth));
                }
            }
        }

        let mut nodes = depths
            .into_iter()
            .map(|(address, depth)| TraceNode {
                address: address.to_string(),
                depth,
            })
            .collect::<Vec<_>>();
        nodes
            .sort_by(|left, right| (left.depth, &left.address).cmp(&(right.depth, &right.address)));

        let mut transfers = edges.into_values().collect::<Vec<_>>();
        transfers.sort_by(|left, right| {
            (left.block_timestamp, &left.id.tx_id, &left.id.event_key).cmp(&(
                right.block_timestamp,
                &right.id.tx_id,
                &right.id.event_key,
            ))
        });
        let edges = transfers.into_iter().map(TraceEdge::from).collect();
        let truncated = !truncation_reasons.is_empty();

        Ok(TraceGraph {
            chain: root.chain(),
            asset_symbol: params.asset.symbol(root.chain()).to_owned(),
            root_address: root.to_string(),
            nodes,
            edges,
            complete: !truncated,
            truncated,
            truncation_reasons,
        })
    }
}

fn matches_direction(direction: Direction, address: &ChainAddress, transfer: &Transfer) -> bool {
    match direction {
        Direction::Incoming => &transfer.to == address,
        Direction::Outgoing => &transfer.from == address,
        Direction::Both => &transfer.from == address || &transfer.to == address,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TraceNode {
    pub address: String,
    pub depth: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TraceEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub chain: crate::domain::Chain,
    pub asset_symbol: String,
    pub attribution: crate::domain::Attribution,
    pub tx_id: String,
    pub event_index: Option<u32>,
    pub id_source: String,
    pub amount_raw: String,
    pub amount_display: String,
    pub token_decimals: u8,
    pub block_timestamp: i64,
    pub confirmed: bool,
}

impl From<Transfer> for TraceEdge {
    fn from(transfer: Transfer) -> Self {
        let id_source = if transfer.event_index.is_some() {
            "provider"
        } else {
            "synthetic"
        };
        Self {
            id: format!(
                "{}:{}:{}",
                transfer.network, transfer.id.tx_id, transfer.id.event_key
            ),
            source: transfer.from.to_string(),
            target: transfer.to.to_string(),
            chain: transfer.chain,
            asset_symbol: transfer.asset_symbol,
            attribution: transfer.attribution,
            tx_id: transfer.id.tx_id,
            event_index: transfer.event_index,
            id_source: id_source.to_owned(),
            amount_raw: transfer.amount.raw_string(),
            amount_display: transfer.amount.display(),
            token_decimals: transfer.amount.decimals(),
            block_timestamp: transfer.block_timestamp,
            confirmed: transfer.confirmed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TraceGraph {
    pub chain: crate::domain::Chain,
    pub asset_symbol: String,
    pub root_address: String,
    pub nodes: Vec<TraceNode>,
    pub edges: Vec<TraceEdge>,
    pub complete: bool,
    pub truncated: bool,
    pub truncation_reasons: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("max_hops must be between 1 and 3")]
    InvalidHops,
    #[error("min_timestamp must not be after max_timestamp")]
    InvalidWindow,
    #[error("trace limits are outside the supported range")]
    InvalidLimits,
    #[error("minimum amount decimals must match the selected asset")]
    InvalidMinimumDecimals,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::{
        domain::{AssetSpec, Chain, TokenAmount, TraceLimits, TraceWindow},
        provider::TransferBatch,
    };

    #[derive(Default)]
    struct FixtureProvider {
        transfers: Mutex<HashMap<String, Vec<Transfer>>>,
        truncated: bool,
    }

    #[async_trait]
    impl TransferProvider for FixtureProvider {
        async fn transfers(
            &self,
            address: &ChainAddress,
            _asset: &AssetSpec,
            _window: TraceWindow,
            _max_pages: usize,
        ) -> Result<TransferBatch, ProviderError> {
            Ok(TransferBatch {
                transfers: self
                    .transfers
                    .lock()
                    .unwrap()
                    .get(address.as_str())
                    .cloned()
                    .unwrap_or_default(),
                truncated: self.truncated,
            })
        }
    }

    fn address(suffix: u8) -> ChainAddress {
        let mut payload = [0_u8; 21];
        payload[0] = 0x41;
        payload[20] = suffix;
        ChainAddress::tron_from_payload(payload).unwrap()
    }

    fn transfer(
        index: u32,
        from: &ChainAddress,
        to: &ChainAddress,
        amount: &str,
        timestamp: i64,
    ) -> Transfer {
        Transfer {
            id: TransferId {
                tx_id: format!("tx-{index:02}"),
                event_key: format!("log-{index}"),
            },
            chain: Chain::Tron,
            network: "tron-mainnet".to_owned(),
            asset_symbol: "USDT".to_owned(),
            attribution: crate::domain::Attribution::Exact,
            from: from.clone(),
            to: to.clone(),
            amount: TokenAmount::parse(amount, 6).unwrap(),
            event_index: Some(index),
            block_timestamp: timestamp,
            confirmed: true,
        }
    }

    fn params(root: ChainAddress, direction: Direction, hops: u8) -> TraceParams {
        TraceParams {
            root,
            asset: AssetSpec::token("USDT", "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t", 6),
            direction,
            max_hops: hops,
            minimum: TokenAmount::parse("1", 6).unwrap(),
            window: TraceWindow {
                min_timestamp: 0,
                max_timestamp: 9_999,
            },
            limits: TraceLimits::default(),
        }
    }

    #[tokio::test]
    async fn outgoing_bfs_terminates_on_cycle_and_keeps_direction() {
        let a = address(1);
        let b = address(2);
        let c = address(3);
        let fixture = FixtureProvider::default();
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(a.to_string(), vec![transfer(1, &a, &b, "1500000", 100)]);
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(b.to_string(), vec![transfer(2, &b, &c, "750000", 200)]);
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(c.to_string(), vec![transfer(3, &c, &a, "100000", 300)]);

        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(params(a.clone(), Direction::Outgoing, 3))
            .await
            .unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.edges[0].source, a.to_string());
        assert_eq!(graph.chain, Chain::Tron);
        assert_eq!(graph.asset_symbol, "USDT");
        assert!(graph.complete);
    }

    #[tokio::test]
    async fn hop_limit_and_minimum_amount_are_enforced() {
        let a = address(1);
        let b = address(2);
        let c = address(3);
        let fixture = FixtureProvider::default();
        fixture.transfers.lock().unwrap().insert(
            a.to_string(),
            vec![
                transfer(1, &a, &b, "1500000", 100),
                transfer(2, &a, &c, "50", 101),
            ],
        );
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(b.to_string(), vec![transfer(3, &b, &c, "750000", 200)]);
        let mut request = params(a, Direction::Outgoing, 1);
        request.minimum = TokenAmount::parse("1000000", 6).unwrap();

        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(request)
            .await
            .unwrap();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].amount_display, "1.5");
    }

    #[tokio::test]
    async fn duplicate_events_are_removed_but_parallel_event_indexes_survive() {
        let a = address(1);
        let b = address(2);
        let first = transfer(1, &a, &b, "1000000", 100);
        let mut parallel = first.clone();
        parallel.id.event_key = "log-2".to_owned();
        parallel.event_index = Some(2);
        let fixture = FixtureProvider::default();
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(a.to_string(), vec![first.clone(), first, parallel]);

        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(params(a, Direction::Outgoing, 1))
            .await
            .unwrap();
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].event_index, Some(1));
        assert_eq!(graph.edges[1].event_index, Some(2));
    }

    #[tokio::test]
    async fn node_limit_reports_truncation() {
        let a = address(1);
        let b = address(2);
        let c = address(3);
        let fixture = FixtureProvider::default();
        fixture.transfers.lock().unwrap().insert(
            a.to_string(),
            vec![
                transfer(1, &a, &b, "100", 100),
                transfer(2, &a, &c, "100", 101),
            ],
        );
        let mut request = params(a, Direction::Outgoing, 1);
        request.limits.max_nodes = 2;
        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(request)
            .await
            .unwrap();
        assert!(graph.truncated);
        assert_eq!(graph.truncation_reasons, vec!["max_nodes"]);
    }

    #[tokio::test]
    async fn rejects_invalid_limits_for_direct_library_callers() {
        let a = address(1);
        let fixture = FixtureProvider::default();
        let mut request = params(a, Direction::Outgoing, 1);
        request.limits.max_nodes = 0;
        let error = TraceEngine::new(Arc::new(fixture))
            .trace(request)
            .await
            .unwrap_err();
        assert!(matches!(error, TraceError::InvalidLimits));
    }

    #[tokio::test]
    async fn rejects_minimum_amount_with_wrong_asset_decimals() {
        let a = address(1);
        let fixture = FixtureProvider::default();
        let mut request = params(a, Direction::Outgoing, 1);
        request.minimum = TokenAmount::parse("1", 8).unwrap();
        let error = TraceEngine::new(Arc::new(fixture))
            .trace(request)
            .await
            .unwrap_err();
        assert!(matches!(error, TraceError::InvalidMinimumDecimals));
    }

    #[tokio::test]
    async fn excludes_provider_rows_outside_the_requested_window() {
        let a = address(1);
        let b = address(2);
        let fixture = FixtureProvider::default();
        fixture.transfers.lock().unwrap().insert(
            a.to_string(),
            vec![
                transfer(1, &a, &b, "1000000", 99),
                transfer(2, &a, &b, "1000000", 100),
                transfer(3, &a, &b, "1000000", 200),
                transfer(4, &a, &b, "1000000", 201),
            ],
        );
        let mut request = params(a, Direction::Outgoing, 1);
        request.window = TraceWindow {
            min_timestamp: 100,
            max_timestamp: 200,
        };
        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(request)
            .await
            .unwrap();
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].block_timestamp, 100);
        assert_eq!(graph.edges[1].block_timestamp, 200);
    }

    #[tokio::test]
    async fn incoming_walks_backwards_and_self_transfer_does_not_expand_twice() {
        let a = address(1);
        let b = address(2);
        let c = address(3);
        let fixture = FixtureProvider::default();
        fixture.transfers.lock().unwrap().insert(
            a.to_string(),
            vec![
                transfer(1, &b, &a, "1000000", 200),
                transfer(4, &a, &a, "100000", 250),
            ],
        );
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(b.to_string(), vec![transfer(2, &c, &b, "900000", 100)]);

        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(params(a, Direction::Incoming, 2))
            .await
            .unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 3);
    }

    #[tokio::test]
    async fn provider_page_truncation_is_preserved_in_graph_status() {
        let a = address(1);
        let b = address(2);
        let fixture = FixtureProvider {
            truncated: true,
            ..FixtureProvider::default()
        };
        fixture
            .transfers
            .lock()
            .unwrap()
            .insert(a.to_string(), vec![transfer(1, &a, &b, "1000000", 100)]);

        let graph = TraceEngine::new(Arc::new(fixture))
            .trace(params(a, Direction::Both, 1))
            .await
            .unwrap();
        assert!(graph.truncated);
        assert_eq!(graph.truncation_reasons, vec!["max_pages_per_address"]);
        assert_eq!(graph.edges.len(), 1);
    }

    #[tokio::test]
    async fn output_is_deterministic_when_provider_order_changes() {
        let a = address(1);
        let b = address(2);
        let c = address(3);
        let first = transfer(1, &a, &b, "1000000", 100);
        let second = transfer(2, &a, &c, "2000000", 200);
        let left = FixtureProvider::default();
        left.transfers
            .lock()
            .unwrap()
            .insert(a.to_string(), vec![second.clone(), first.clone()]);
        let right = FixtureProvider::default();
        right
            .transfers
            .lock()
            .unwrap()
            .insert(a.to_string(), vec![first, second]);

        let left_graph = TraceEngine::new(Arc::new(left))
            .trace(params(a.clone(), Direction::Outgoing, 1))
            .await
            .unwrap();
        let right_graph = TraceEngine::new(Arc::new(right))
            .trace(params(a, Direction::Outgoing, 1))
            .await
            .unwrap();
        assert_eq!(left_graph, right_graph);
    }
}
