use std::fmt;

use num_bigint::BigUint;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Chain {
    Bitcoin,
    Ethereum,
    EthereumClassic,
    Solana,
    Tron,
    Bnb,
    Polygon,
    Arbitrum,
    Optimism,
    Base,
    Avalanche,
    Dogecoin,
    Litecoin,
    Xrp,
}

impl Chain {
    pub const ALL: [Self; 14] = [
        Self::Bitcoin,
        Self::Ethereum,
        Self::EthereumClassic,
        Self::Solana,
        Self::Tron,
        Self::Bnb,
        Self::Polygon,
        Self::Arbitrum,
        Self::Optimism,
        Self::Base,
        Self::Avalanche,
        Self::Dogecoin,
        Self::Litecoin,
        Self::Xrp,
    ];

    pub fn family(self) -> ChainFamily {
        match self {
            Self::Bitcoin | Self::Dogecoin | Self::Litecoin => ChainFamily::Utxo,
            Self::Ethereum
            | Self::EthereumClassic
            | Self::Bnb
            | Self::Polygon
            | Self::Arbitrum
            | Self::Optimism
            | Self::Base
            | Self::Avalanche => ChainFamily::Evm,
            Self::Solana => ChainFamily::Solana,
            Self::Tron => ChainFamily::Tron,
            Self::Xrp => ChainFamily::Xrpl,
        }
    }

    pub fn network_name(self) -> &'static str {
        match self {
            Self::Bitcoin => "bitcoin-mainnet",
            Self::Ethereum => "ethereum-mainnet",
            Self::EthereumClassic => "ethereum-classic-mainnet",
            Self::Solana => "solana-mainnet",
            Self::Tron => "tron-mainnet",
            Self::Bnb => "bnb-smart-chain",
            Self::Polygon => "polygon-pos",
            Self::Arbitrum => "arbitrum-one",
            Self::Optimism => "op-mainnet",
            Self::Base => "base-mainnet",
            Self::Avalanche => "avalanche-c-chain",
            Self::Dogecoin => "dogecoin-mainnet",
            Self::Litecoin => "litecoin-mainnet",
            Self::Xrp => "xrpl-mainnet",
        }
    }

    pub fn native_symbol(self) -> &'static str {
        match self {
            Self::Bitcoin => "BTC",
            Self::Ethereum | Self::Arbitrum | Self::Optimism | Self::Base => "ETH",
            Self::EthereumClassic => "ETC",
            Self::Solana => "SOL",
            Self::Tron => "TRX",
            Self::Bnb => "BNB",
            Self::Polygon => "POL",
            Self::Avalanche => "AVAX",
            Self::Dogecoin => "DOGE",
            Self::Litecoin => "LTC",
            Self::Xrp => "XRP",
        }
    }

    pub fn native_decimals(self) -> u8 {
        match self {
            Self::Bitcoin | Self::Dogecoin | Self::Litecoin => 8,
            Self::Ethereum
            | Self::EthereumClassic
            | Self::Bnb
            | Self::Polygon
            | Self::Arbitrum
            | Self::Optimism
            | Self::Base
            | Self::Avalanche => 18,
            Self::Solana => 9,
            Self::Tron | Self::Xrp => 6,
        }
    }

    pub fn evm_chain_id(self) -> Option<u64> {
        match self {
            Self::Ethereum => Some(1),
            Self::EthereumClassic => Some(61),
            Self::Bnb => Some(56),
            Self::Polygon => Some(137),
            Self::Arbitrum => Some(42_161),
            Self::Optimism => Some(10),
            Self::Base => Some(8_453),
            Self::Avalanche => Some(43_114),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainFamily {
    Utxo,
    Evm,
    Solana,
    Tron,
    Xrpl,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChainAddress {
    chain: Chain,
    value: String,
}

impl ChainAddress {
    pub fn parse(chain: Chain, value: &str) -> Result<Self, AddressError> {
        if value.len() > 128 {
            return Err(AddressError::InvalidForChain(chain));
        }
        let normalized = value.trim();
        let valid = match chain.family() {
            ChainFamily::Evm => validate_evm(normalized),
            ChainFamily::Solana => validate_solana(normalized),
            ChainFamily::Tron => validate_tron(normalized),
            ChainFamily::Xrpl => validate_xrp(normalized),
            ChainFamily::Utxo => validate_utxo(chain, normalized),
        };
        if !valid {
            return Err(AddressError::InvalidForChain(chain));
        }
        let value = if chain.family() == ChainFamily::Evm
            || matches!(chain, Chain::Bitcoin | Chain::Litecoin)
                && normalized.to_ascii_lowercase().starts_with(match chain {
                    Chain::Bitcoin => "bc1",
                    Chain::Litecoin => "ltc1",
                    _ => unreachable!(),
                }) {
            normalized.to_ascii_lowercase()
        } else {
            normalized.to_owned()
        };
        Ok(Self { chain, value })
    }

    pub fn tron_from_payload(payload: [u8; 21]) -> Result<Self, AddressError> {
        if payload[0] != 0x41 {
            return Err(AddressError::InvalidForChain(Chain::Tron));
        }
        let mut bytes = payload.to_vec();
        bytes.extend_from_slice(&checksum(&payload));
        Self::parse(Chain::Tron, &bs58::encode(bytes).into_string())
    }

    pub fn chain(&self) -> Chain {
        self.chain
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for ChainAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.value)
    }
}

impl Serialize for ChainAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

fn validate_evm(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_solana(value: &str) -> bool {
    bs58::decode(value)
        .into_vec()
        .map(|decoded| decoded.len() == 32)
        .unwrap_or(false)
}

fn validate_tron(value: &str) -> bool {
    let Ok(decoded) = bs58::decode(value).into_vec() else {
        return false;
    };
    decoded.len() == 25
        && decoded[0] == 0x41
        && checksum(&decoded[..21]).as_slice() == &decoded[21..]
}

fn validate_xrp(value: &str) -> bool {
    const RIPPLE_ALPHABET: &str = "rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";
    let Some(decoded) = decode_base58(value, RIPPLE_ALPHABET) else {
        return false;
    };
    decoded.len() == 25 && decoded[0] == 0 && checksum(&decoded[..21]).as_slice() == &decoded[21..]
}

fn validate_utxo(chain: Chain, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let hrp = match chain {
        Chain::Bitcoin => Some("bc"),
        Chain::Litecoin => Some("ltc"),
        Chain::Dogecoin => None,
        _ => return false,
    };
    if let Some(expected_hrp) = hrp {
        if lower.starts_with(&format!("{expected_hrp}1")) {
            return validate_bech32(value, expected_hrp);
        }
    }
    let Ok(decoded) = bs58::decode(value).into_vec() else {
        return false;
    };
    if decoded.len() != 25 || checksum(&decoded[..21]).as_slice() != &decoded[21..] {
        return false;
    }
    match chain {
        Chain::Bitcoin => matches!(decoded[0], 0x00 | 0x05),
        Chain::Dogecoin => matches!(decoded[0], 0x1e | 0x16),
        Chain::Litecoin => matches!(decoded[0], 0x30 | 0x32 | 0x05),
        _ => false,
    }
}

fn validate_bech32(value: &str, expected_hrp: &str) -> bool {
    if value.len() < 14 || value.len() > 90 {
        return false;
    }
    let has_lower = value.bytes().any(|byte| byte.is_ascii_lowercase());
    let has_upper = value.bytes().any(|byte| byte.is_ascii_uppercase());
    if has_lower && has_upper {
        return false;
    }
    let normalized = value.to_ascii_lowercase();
    let Some(separator) = normalized.rfind('1') else {
        return false;
    };
    if &normalized[..separator] != expected_hrp || separator + 7 > normalized.len() {
        return false;
    }
    const CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
    let mut values = Vec::with_capacity(normalized.len() - separator - 1);
    for character in normalized[separator + 1..].chars() {
        let Some(index) = CHARSET.find(character) else {
            return false;
        };
        values.push(index as u8);
    }
    if values.len() < 7 {
        return false;
    }
    let witness_version = values[0];
    if witness_version > 16 {
        return false;
    }
    let Some(program) = convert_bits_5_to_8(&values[1..values.len() - 6]) else {
        return false;
    };
    if !(2..=40).contains(&program.len())
        || witness_version == 0 && !matches!(program.len(), 20 | 32)
    {
        return false;
    }
    let polymod = bech32_polymod(
        expected_hrp
            .bytes()
            .map(|byte| byte >> 5)
            .chain(std::iter::once(0))
            .chain(expected_hrp.bytes().map(|byte| byte & 31))
            .chain(values.iter().copied()),
    );
    if witness_version == 0 {
        polymod == 1
    } else {
        polymod == 0x2bc8_30a3
    }
}

fn convert_bits_5_to_8(values: &[u8]) -> Option<Vec<u8>> {
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    let mut output = Vec::new();
    for value in values {
        if *value > 31 {
            return None;
        }
        accumulator = ((accumulator & 0x0fff) << 5) | u32::from(*value);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
        }
    }
    if bits >= 5 || ((accumulator << (8 - bits)) & 0xff) != 0 {
        return None;
    }
    Some(output)
}

fn bech32_polymod(values: impl IntoIterator<Item = u8>) -> u32 {
    const GENERATORS: [u32; 5] = [
        0x3b6a_57b2,
        0x2650_8e6d,
        0x1ea1_19fa,
        0x3d42_33dd,
        0x2a14_62b3,
    ];
    let mut checksum = 1_u32;
    for value in values {
        let top = checksum >> 25;
        checksum = ((checksum & 0x01ff_ffff) << 5) ^ u32::from(value);
        for (index, generator) in GENERATORS.iter().enumerate() {
            if ((top >> index) & 1) != 0 {
                checksum ^= generator;
            }
        }
    }
    checksum
}

fn decode_base58(value: &str, alphabet: &str) -> Option<Vec<u8>> {
    let alphabet = alphabet.as_bytes();
    let mut decoded = vec![0_u8];
    for character in value.bytes() {
        let mut carry = alphabet
            .iter()
            .position(|candidate| *candidate == character)? as u32;
        for byte in decoded.iter_mut().rev() {
            let accumulator = u32::from(*byte) * 58 + carry;
            *byte = (accumulator & 0xff) as u8;
            carry = accumulator >> 8;
        }
        while carry > 0 {
            decoded.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let leading_zeroes = value
        .bytes()
        .take_while(|byte| *byte == alphabet[0])
        .count();
    let first_nonzero = decoded
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(decoded.len());
    let mut output = vec![0_u8; leading_zeroes];
    output.extend_from_slice(&decoded[first_nonzero..]);
    Some(output)
}

fn checksum(payload: &[u8]) -> [u8; 4] {
    let first = Sha256::digest(payload);
    let second = Sha256::digest(first);
    [second[0], second[1], second[2], second[3]]
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum AddressError {
    #[error("address is not valid for {0:?}")]
    InvalidForChain(Chain),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssetSpec {
    Native,
    Token {
        symbol: String,
        contract_address: String,
        decimals: u8,
    },
}

impl AssetSpec {
    pub fn native() -> Self {
        Self::Native
    }

    pub fn token(
        symbol: impl Into<String>,
        contract_address: impl Into<String>,
        decimals: u8,
    ) -> Self {
        Self::Token {
            symbol: symbol.into(),
            contract_address: contract_address.into(),
            decimals,
        }
    }

    pub fn symbol(&self, chain: Chain) -> &str {
        match self {
            Self::Native => chain.native_symbol(),
            Self::Token { symbol, .. } => symbol,
        }
    }

    pub fn decimals(&self, chain: Chain) -> u8 {
        match self {
            Self::Native => chain.native_decimals(),
            Self::Token { decimals, .. } => *decimals,
        }
    }

    pub fn contract_address(&self) -> Option<&str> {
        match self {
            Self::Native => None,
            Self::Token {
                contract_address, ..
            } => Some(contract_address),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenAmount {
    raw: BigUint,
    decimals: u8,
}

impl TokenAmount {
    pub fn parse(raw: &str, decimals: u8) -> Result<Self, AmountError> {
        if raw.is_empty() || raw.len() > 128 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AmountError::InvalidRawAmount);
        }
        let raw = BigUint::parse_bytes(raw.as_bytes(), 10).ok_or(AmountError::InvalidRawAmount)?;
        Ok(Self { raw, decimals })
    }

    pub fn raw(&self) -> &BigUint {
        &self.raw
    }

    pub fn raw_string(&self) -> String {
        self.raw.to_str_radix(10)
    }

    pub fn decimals(&self) -> u8 {
        self.decimals
    }

    pub fn display(&self) -> String {
        format_units(&self.raw_string(), self.decimals)
    }
}

fn format_units(raw: &str, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_owned();
    }
    let decimals = decimals as usize;
    let (whole, fraction) = if raw.len() > decimals {
        let split = raw.len() - decimals;
        (&raw[..split], raw[split..].to_owned())
    } else {
        ("0", format!("{}{}", "0".repeat(decimals - raw.len()), raw))
    };
    let trimmed = fraction.trim_end_matches('0');
    if trimmed.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}.{trimmed}")
    }
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum AmountError {
    #[error("raw token amount must be an unsigned base-10 integer")]
    InvalidRawAmount,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TransferId {
    pub tx_id: String,
    pub event_key: String,
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: TransferId,
    pub chain: Chain,
    pub network: String,
    pub asset_symbol: String,
    pub attribution: Attribution,
    pub from: ChainAddress,
    pub to: ChainAddress,
    pub amount: TokenAmount,
    pub event_index: Option<u32>,
    pub block_timestamp: i64,
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    Exact,
    ProviderNormalized,
    ProportionalHeuristic,
}

#[derive(Clone, Copy, Debug)]
pub struct TraceWindow {
    pub min_timestamp: i64,
    pub max_timestamp: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct TraceLimits {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_pages_per_address: usize,
}

impl Default for TraceLimits {
    fn default() -> Self {
        Self {
            max_nodes: 250,
            max_edges: 1_000,
            max_pages_per_address: 10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TraceParams {
    pub root: ChainAddress,
    pub asset: AssetSpec,
    pub direction: Direction,
    pub max_hops: u8,
    pub minimum: TokenAmount,
    pub window: TraceWindow,
    pub limits: TraceLimits,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_addresses_across_five_chain_families() {
        assert!(ChainAddress::parse(Chain::Bitcoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC").is_ok());
        assert!(ChainAddress::parse(Chain::Dogecoin, "DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY").is_ok());
        assert!(ChainAddress::parse(Chain::Litecoin, "LWrzZrgBvieg6SFK2GbwtZDDAy8hsHoMLL").is_ok());
        assert!(ChainAddress::parse(
            Chain::Ethereum,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624"
        )
        .is_ok());
        assert!(ChainAddress::parse(
            Chain::EthereumClassic,
            "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624"
        )
        .is_ok());
        assert!(ChainAddress::parse(
            Chain::Solana,
            "B7eKLu5DFf5bafEKpZPyBFuoBmMZtuworcbqDSaoPPDP"
        )
        .is_ok());
        assert!(ChainAddress::parse(Chain::Tron, "TMKhcf7v9tt1Tnkxrrbr6CNZHMiwxrsgAd").is_ok());
        assert!(ChainAddress::parse(Chain::Xrp, "rGzDj59ipb1tQjoWmJ2DsLDypuaZ6f9EhC").is_ok());
        assert!(
            ChainAddress::parse(Chain::Bitcoin, "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh")
                .is_ok()
        );
    }

    #[test]
    fn rejects_cross_chain_and_checksum_mismatches() {
        assert!(
            ChainAddress::parse(Chain::Dogecoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC").is_err()
        );
        assert!(ChainAddress::parse(Chain::Bitcoin, "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQS1").is_err());
        assert!(
            ChainAddress::parse(Chain::Solana, "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624")
                .is_err()
        );
        assert!(
            ChainAddress::parse(Chain::Bitcoin, "bc13qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq0hg8yd")
                .is_err()
        );
        assert!(
            ChainAddress::parse(Chain::Bitcoin, "bc1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqs9wcxj")
                .is_err()
        );
    }

    #[test]
    fn canonicalizes_uppercase_segwit_addresses() {
        let lower =
            ChainAddress::parse(Chain::Bitcoin, "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh")
                .unwrap();
        let upper =
            ChainAddress::parse(Chain::Bitcoin, "BC1QXY2KGDYGJRSQTZQ2N0YRF2493P83KKFJHX0WLH")
                .unwrap();
        assert_eq!(lower, upper);
    }

    #[test]
    fn token_amount_keeps_integer_precision() {
        let amount = TokenAmount::parse("1500000", 6).unwrap();
        assert_eq!(amount.display(), "1.5");
        let huge = TokenAmount::parse("340282366920938463463374607431768211456", 6).unwrap();
        assert_eq!(huge.raw_string(), "340282366920938463463374607431768211456");
    }

    #[test]
    fn rejects_unreasonably_long_addresses_and_amounts_before_decoding() {
        assert!(ChainAddress::parse(Chain::Solana, &"1".repeat(129)).is_err());
        assert!(ChainAddress::parse(Chain::Bitcoin, &"1".repeat(129)).is_err());
        assert_eq!(
            TokenAmount::parse(&"9".repeat(129), 8),
            Err(AmountError::InvalidRawAmount)
        );
    }

    #[test]
    fn native_assets_use_chain_specific_decimals() {
        assert_eq!(AssetSpec::native().decimals(Chain::Bitcoin), 8);
        assert_eq!(AssetSpec::native().decimals(Chain::Ethereum), 18);
        assert_eq!(AssetSpec::native().decimals(Chain::EthereumClassic), 18);
        assert_eq!(AssetSpec::native().symbol(Chain::EthereumClassic), "ETC");
        assert_eq!(AssetSpec::native().decimals(Chain::Solana), 9);
        assert_eq!(AssetSpec::native().symbol(Chain::Xrp), "XRP");
    }

    #[test]
    fn ethereum_classic_has_a_stable_public_identity() {
        assert_eq!(Chain::ALL.len(), 14);
        assert!(Chain::ALL.contains(&Chain::EthereumClassic));
        assert_eq!(Chain::EthereumClassic.family(), ChainFamily::Evm);
        assert_eq!(
            Chain::EthereumClassic.network_name(),
            "ethereum-classic-mainnet"
        );
        assert_eq!(Chain::EthereumClassic.evm_chain_id(), Some(61));
        assert_eq!(
            serde_json::to_string(&Chain::EthereumClassic).unwrap(),
            "\"ethereum_classic\""
        );
    }
}
