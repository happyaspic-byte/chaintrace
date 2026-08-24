"use client";

import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { isValidAddress } from "@/lib/address-validation.mjs";

type NodeKind = "focus" | "exchange" | "wallet" | "contract" | "bridge";
type ChainFamily = "utxo" | "evm" | "solana" | "tron" | "xrpl";
type ChainId =
  | "bitcoin"
  | "ethereum"
  | "solana"
  | "tron"
  | "bnb"
  | "polygon"
  | "arbitrum"
  | "optimism"
  | "base"
  | "avalanche"
  | "dogecoin"
  | "litecoin"
  | "xrp";

type ChainConfig = {
  id: ChainId;
  name: string;
  network: string;
  family: ChainFamily;
  badge: string;
  assets: string[];
  addresses: string[];
  amountScale: number;
  minimum: number;
};

type FlowNode = {
  id: string;
  address: string;
  label: string;
  tag: string;
  kind: NodeKind;
  x: number;
  y: number;
  balance: string;
  firstSeen: string;
  transactions: number;
  note: string;
};

type FlowEdge = {
  id: string;
  from: string;
  to: string;
  baseAmount: number;
  time: string;
  tx: string;
  direction: "in" | "out";
  minHops: 1 | 2 | 3;
  ageDays: number;
};

const BTC_ADDRESSES = [
  "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC", "163ApeUHNC3dYEbZneW1STQbp1dAvTk9JM",
  "1CDASWk8fywQRRVsSVgQzLgW1NbdYAFNbe", "1FRYPiH5Ry6y1N1org3SiMUeHtLaFQVgZh",
  "1EFV9ZBdYc5dRUMLZFsg39FWDxv6rvDqu2", "1FBPYA4V6tv8nmJYLS2ebdfxLGU9QKm1Bx",
  "1AMzqhdeb6HLH2FsHoaCgUJYjAsqivAZDT", "13ea2iK8PttXBKSFoipwr5vB3F9Saoa1xV",
];
const DOGE_ADDRESSES = [
  "DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY", "DFJfM8QrSJAqYTGDt7GNc24bHpuYZmUseB",
  "DJMP3imQJgFNUdfrybreZ163z5tZvTi5CX", "D5z8GCQKCd4Z4UFGgqoC7woP3rLargsH9X",
  "DRTS41V2Qpo5439ksPYHMBfhwZUiHbQTjk", "DJb5oXDkmgvJJE5ViREiNx4GJbX7MzkmuN",
  "D6oNnbmUi1ppxNJSWTv23hDM1X2fCiJkeg", "DF8iMuNZrcwPdwSkuH3reSMkAGxjiE8Rrh",
];
const LTC_ADDRESSES = [
  "LWrzZrgBvieg6SFK2GbwtZDDAy8hsHoMLL", "LZ1PzNWetRGaSvETJXUNJH7shWiDu27YSU",
  "LeoqAnK3yuy4KyS9CgEB6vEvQNLM1gmYFF", "LbwBcHNyXUQF3Vfit7RZRwDrgEWsxZGHSK",
  "LX1hR3LAXBVGBWpXWBLxxjj8QX6dz2Xxbt", "Le6McMo8PvCnE65PK9RGprnLmob7eZUj5W",
  "LhYuKZQg71QT1nxFQGbp3XvBRyimLatRJT", "LMdDMTLpuNB5Bd5tGpsE2fP51nQETVbenp",
];
const XRP_ADDRESSES = [
  "rGeNbpGQemfWPP9RxY49DptwESwNsiSGR", "rfqaEpsD6fEFAwS2Jijv8dEpSKHZaAwbiQ",
  "rGzDj59ipb1tQjoWmJ2DsLDypuaZ6f9EhC", "rM5VwQgwVWCS8Q4mWQ1xC4E9STQd9Z7Wvu",
  "rUEtsSg7yP6Hko6MXv9494d4FtCuW4ks4r", "rNpMiKvi5Mv7nkb9mt85BkLEd9q32eDrV6",
  "rhduMKTJH9TypbLKXhKrSSQXfN7oXhidvK", "raJmQMS9PPm8VLEuHDHg9pUroqSCxyBE2A",
];
const SOL_ADDRESSES = [
  "B7eKLu5DFf5bafEKpZPyBFuoBmMZtuworcbqDSaoPPDP", "47ddKiMQ9esPLDZQJWVE4NUe2ua7A7PJZJpmdbTTcA9g",
  "AAQuRPZm5QSDdVqFWJ6Xx6Bqpctdp2CUxrvCGWab85dU", "EecisrPihYGgS2AooxGLGp4m8h8ct8pMdWha3aGY3G6u",
  "9VRaQaqMSuhZqhzVZL9DUX8vHqrTub5hjx2Tk1g9GLhM", "Aufh3GTyL2nkYYn8YDLKXUvqg514fnC2kSbHgS9sDrG2",
  "GGhbhJZesTgS6PJ4wnDMaKiRtBZzXrLoK4bninLYmRGM", "Dqw8bfSqdqtKHdr21N2cacHdjVbKpTEon5yb5M1NuhTy",
];
const TRON_ADDRESSES = [
  "TEYtcgvzB8uJnYsGRT5ZnM2Cvr8ZF8d9jZ", "THwJ7g2TAWtf8Ap7prcA341uxzfrnPizyd",
  "TMKhcf7v9tt1Tnkxrrbr6CNZHMiwxrsgAd", "TQi77eDP9GsMFg27viKBUFLtWcjy6X85ch",
  "TU6WcdJqKbxdkLHX6DtVrPhLTDXDjRWeRU", "TXUturgtV6DcciZoJjQCKg7kN384URG5Hy",
  "TBSQkYMkM7WPcMnKcruFrL4i6hy2H3ar32", "TEppFXTDLVVjwyjB2GSwqQX3uutQPWvgVG",
];
const EVM_ADDRESSES = [
  "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624", "0x149389ff0a7824d3172cad6c596498f384c1798e",
  "0x23954cfab0d22e650746cd2cdbf56eb326123325", "0x95a0776226f24d44838f5dcb14417e0b1126a1c2",
  "0xe85fdf8e19fe673879e9814e4359ee84f848fcb0", "0xae88687ac302c643136906c865a951e67fb1c859",
  "0xbcf81d25575b4d2f472cda79689895c0ec90856a", "0x0e4b686ce8190e3f4c128808666f0dd353522cf6",
];

const CHAINS: ChainConfig[] = [
  { id: "bitcoin", name: "Bitcoin", network: "Bitcoin Mainnet", family: "utxo", badge: "BTC", assets: ["BTC"], addresses: BTC_ADDRESSES, amountScale: 0.00001, minimum: 0.01 },
  { id: "ethereum", name: "Ethereum", network: "Ethereum Mainnet", family: "evm", badge: "ETH", assets: ["ETH", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.0001, minimum: 0.1 },
  { id: "solana", name: "Solana", network: "Solana Mainnet", family: "solana", badge: "SOL", assets: ["SOL", "USDC", "USDT"], addresses: SOL_ADDRESSES, amountScale: 0.01, minimum: 10 },
  { id: "tron", name: "TRON", network: "TRON Mainnet", family: "tron", badge: "TRX", assets: ["USDT", "TRX"], addresses: TRON_ADDRESSES, amountScale: 1, minimum: 1000 },
  { id: "bnb", name: "BNB Chain", network: "BNB Smart Chain", family: "evm", badge: "BNB", assets: ["BNB", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.0001, minimum: 0.1 },
  { id: "polygon", name: "Polygon", network: "Polygon PoS", family: "evm", badge: "POL", assets: ["POL", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.01, minimum: 10 },
  { id: "arbitrum", name: "Arbitrum", network: "Arbitrum One", family: "evm", badge: "ARB", assets: ["ETH", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.0001, minimum: 0.1 },
  { id: "optimism", name: "Optimism", network: "OP Mainnet", family: "evm", badge: "OP", assets: ["ETH", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.0001, minimum: 0.1 },
  { id: "base", name: "Base", network: "Base Mainnet", family: "evm", badge: "BASE", assets: ["ETH", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.0001, minimum: 0.1 },
  { id: "avalanche", name: "Avalanche", network: "Avalanche C-Chain", family: "evm", badge: "AVAX", assets: ["AVAX", "USDT", "USDC"], addresses: EVM_ADDRESSES, amountScale: 0.001, minimum: 1 },
  { id: "dogecoin", name: "Dogecoin", network: "Dogecoin Mainnet", family: "utxo", badge: "DOGE", assets: ["DOGE"], addresses: DOGE_ADDRESSES, amountScale: 10, minimum: 1000 },
  { id: "litecoin", name: "Litecoin", network: "Litecoin Mainnet", family: "utxo", badge: "LTC", assets: ["LTC"], addresses: LTC_ADDRESSES, amountScale: 0.001, minimum: 1 },
  { id: "xrp", name: "XRP Ledger", network: "XRP Ledger Mainnet", family: "xrpl", badge: "XRP", assets: ["XRP"], addresses: XRP_ADDRESSES, amountScale: 10, minimum: 1000 },
];

const NODE_TEMPLATES = [
  { id: "origin-a", label: "Exchange cluster", tag: "거래소 추정", kind: "exchange" as const, x: 118, y: 154, firstSeen: "2024. 03. 18", transactions: 8492, note: "다수 입출금과 반복적인 집금 패턴이 관찰됩니다." },
  { id: "origin-b", label: "Source wallet", tag: "일반 지갑", kind: "wallet" as const, x: 142, y: 468, firstSeen: "2025. 11. 02", transactions: 184, note: "추적 대상과 최근 30일간 여러 차례 거래했습니다." },
  { id: "focus", label: "Investigated wallet", tag: "추적 대상", kind: "focus" as const, x: 450, y: 314, firstSeen: "2026. 07. 21", transactions: 37, note: "현재 분석 중인 중심 주소입니다. 공개 온체인 데이터만 표시합니다." },
  { id: "deposit", label: "Deposit wallet", tag: "입금 지갑", kind: "wallet" as const, x: 726, y: 116, firstSeen: "2026. 08. 08", transactions: 12, note: "짧은 시간 안에 상위 집금 주소로 이동한 연결이 보입니다." },
  { id: "unknown", label: "Unknown wallet", tag: "미분류", kind: "wallet" as const, x: 752, y: 326, firstSeen: "2026. 05. 14", transactions: 93, note: "소유자 라벨이 확인되지 않은 외부 주소입니다." },
  { id: "router", label: "Protocol router", tag: "프로토콜", kind: "contract" as const, x: 656, y: 544, firstSeen: "2023. 09. 27", transactions: 245817, note: "스왑 또는 프로그램 호출이 감지된 주소입니다." },
  { id: "collector", label: "Collector wallet", tag: "집금 지갑", kind: "exchange" as const, x: 928, y: 206, firstSeen: "2022. 01. 09", transactions: 48325, note: "여러 입금 주소의 자산이 반복적으로 모이는 주소입니다." },
  { id: "bridge", label: "Bridge endpoint", tag: "브리지", kind: "bridge" as const, x: 928, y: 464, firstSeen: "2024. 12. 12", transactions: 98214, note: "다른 네트워크로 이어지는 브리지 연관 주소입니다." },
];

const EDGE_TEMPLATES: FlowEdge[] = [
  { id: "e1", from: "origin-a", to: "focus", baseAmount: 128400, time: "08-23 09:42", tx: "9e0a...82fc", direction: "in", minHops: 1, ageDays: 1 },
  { id: "e2", from: "origin-b", to: "focus", baseAmount: 42100, time: "08-21 11:06", tx: "c415...2a70", direction: "in", minHops: 1, ageDays: 3 },
  { id: "e3", from: "focus", to: "deposit", baseAmount: 96250, time: "08-19 11:18", tx: "81bd...ee19", direction: "out", minHops: 1, ageDays: 5 },
  { id: "e4", from: "focus", to: "unknown", baseAmount: 54200, time: "08-15 11:26", tx: "b03c...d743", direction: "out", minHops: 1, ageDays: 9 },
  { id: "e5", from: "focus", to: "router", baseAmount: 12800, time: "08-10 11:41", tx: "a722...408d", direction: "out", minHops: 1, ageDays: 14 },
  { id: "e6", from: "deposit", to: "collector", baseAmount: 95800, time: "08-03 11:32", tx: "f665...909c", direction: "out", minHops: 2, ageDays: 21 },
  { id: "e7", from: "unknown", to: "bridge", baseAmount: 48100, time: "07-20 14:09", tx: "2cc0...41ba", direction: "out", minHops: 2, ageDays: 35 },
  { id: "e8", from: "router", to: "bridge", baseAmount: 12150, time: "06-30 14:13", tx: "66da...051b", direction: "out", minHops: 2, ageDays: 55 },
  { id: "e9", from: "bridge", to: "origin-a", baseAmount: 8300, time: "08-22 18:06", tx: "71df...aa04", direction: "out", minHops: 3, ageDays: 2 },
];

const KIND_LABEL: Record<NodeKind, string> = { focus: "추적 대상", exchange: "거래소·집금", wallet: "일반 지갑", contract: "컨트랙트·프로그램", bridge: "브리지" };
const CHAIN_MAP = Object.fromEntries(CHAINS.map((chain) => [chain.id, chain])) as Record<ChainId, ChainConfig>;

const formatAmount = (value: number) => new Intl.NumberFormat("ko-KR", { maximumFractionDigits: 5 }).format(value);
const shortAddress = (value: string) => value.length < 18 ? value : `${value.slice(0, 8)}…${value.slice(-7)}`;
const nodeIcon = (kind: NodeKind) => kind === "focus" ? "◎" : kind === "exchange" ? "▦" : kind === "contract" ? "⌘" : kind === "bridge" ? "⇄" : "◇";

export default function Home() {
  const [chainId, setChainId] = useState<ChainId>("bitcoin");
  const chain = CHAIN_MAP[chainId];
  const [asset, setAsset] = useState(chain.assets[0]);
  const [address, setAddress] = useState(chain.addresses[2]);
  const [tracedAddress, setTracedAddress] = useState(chain.addresses[2]);
  const [addressError, setAddressError] = useState(false);
  const [direction, setDirection] = useState<"all" | "in" | "out">("all");
  const [hops, setHops] = useState(2);
  const [minimum, setMinimum] = useState(chain.minimum);
  const [selectedId, setSelectedId] = useState("focus");
  const [isTracing, setIsTracing] = useState(false);
  const [periodDays, setPeriodDays] = useState(30);
  const [zoom, setZoom] = useState(1);
  const [showAllActivities, setShowAllActivities] = useState(false);
  const [notice, setNotice] = useState("13개 네트워크의 결정적 데모 데이터가 준비되었습니다.");
  const traceTimeoutRef = useRef<number | null>(null);
  const traceRequestIdRef = useRef(0);

  useEffect(() => () => {
    if (traceTimeoutRef.current !== null) window.clearTimeout(traceTimeoutRef.current);
  }, []);

  const nodes = useMemo<FlowNode[]>(() => NODE_TEMPLATES.map((node, index) => ({
    ...node,
    address: node.id === "focus" ? tracedAddress : chain.addresses[index],
    balance: node.kind === "contract" || node.kind === "bridge" ? "Protocol" : `${formatAmount((index + 1) * 4820 * chain.amountScale)} ${asset}`,
  })), [chain, asset, tracedAddress]);
  const edges = useMemo(() => EDGE_TEMPLATES.map((edge) => ({ ...edge, amount: edge.baseAmount * chain.amountScale })), [chain]);
  const selectedNode = nodes.find((node) => node.id === selectedId) ?? nodes[2];
  const visibleEdges = useMemo(() => edges.filter((edge) => (direction === "all" || edge.direction === direction) && edge.amount >= minimum && edge.minHops <= hops && edge.ageDays <= periodDays), [edges, direction, minimum, hops, periodDays]);
  const visibleNodeIds = useMemo(() => {
    const ids = new Set(["focus"]);
    visibleEdges.forEach((edge) => { ids.add(edge.from); ids.add(edge.to); });
    return ids;
  }, [visibleEdges]);
  const graphEdges = visibleEdges.filter((edge) => visibleNodeIds.has(edge.from) && visibleNodeIds.has(edge.to));
  const totalFlow = graphEdges.reduce((sum, edge) => sum + edge.amount, 0);
  const selectedActivities = graphEdges.filter((edge) => edge.from === selectedNode.id || edge.to === selectedNode.id);

  function cancelPendingTrace() {
    traceRequestIdRef.current += 1;
    if (traceTimeoutRef.current !== null) {
      window.clearTimeout(traceTimeoutRef.current);
      traceTimeoutRef.current = null;
      setNotice("입력 또는 분석 조건이 변경되어 이전 분석 요청을 취소했습니다.");
    }
    setIsTracing(false);
  }

  function selectChain(nextId: ChainId) {
    cancelPendingTrace();
    const next = CHAIN_MAP[nextId];
    setChainId(nextId);
    setAsset(next.assets[0]);
    setAddress(next.addresses[2]);
    setTracedAddress(next.addresses[2]);
    setAddressError(false);
    setMinimum(next.minimum);
    setDirection("all");
    setHops(2);
    setSelectedId("focus");
    setPeriodDays(30);
    setZoom(1);
    setShowAllActivities(false);
    setNotice(`${next.name} 데모 그래프를 불러왔습니다.`);
  }

  async function handleTrace(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    cancelPendingTrace();
    const requestId = traceRequestIdRef.current;
    setIsTracing(true);
    setNotice(
      chain.family === "evm"
        ? `${chain.name} 주소의 길이와 16진수 형식을 검증하고 있습니다…`
        : `${chain.name} 주소의 checksum과 네트워크 형식을 검증하고 있습니다…`,
    );
    if (!(await isValidAddress(chain.family, chain.id, address))) {
      if (requestId !== traceRequestIdRef.current) return;
      setIsTracing(false);
      setAddressError(true);
      setNotice(`${chain.name} 주소 형식을 확인해주세요. 데모 주소를 다시 불러올 수 있습니다.`);
      return;
    }
    if (requestId !== traceRequestIdRef.current) return;
    setAddressError(false);
    setNotice(`${chain.name}의 최근 ${periodDays}일 확정 거래와 ${asset} 전송을 분석하고 있습니다…`);
    traceTimeoutRef.current = window.setTimeout(() => {
      if (requestId !== traceRequestIdRef.current) return;
      traceTimeoutRef.current = null;
      setIsTracing(false);
      setTracedAddress(address);
      setNotice(`분석 완료 · ${graphEdges.length}건의 ${chain.name} 연결 거래를 찾았습니다.`);
      setSelectedId("focus");
    }, 650);
  }

  function loadDemo() {
    cancelPendingTrace();
    setAddress(chain.addresses[2]); setTracedAddress(chain.addresses[2]); setAddressError(false);
    setDirection("all"); setHops(2); setMinimum(chain.minimum);
    setSelectedId("focus"); setPeriodDays(30); setZoom(1); setShowAllActivities(false);
    setNotice(`${chain.name} 데모 데이터가 다시 로드되었습니다.`);
  }

  async function copyAddress() {
    try {
      await navigator.clipboard.writeText(selectedNode.address);
      setNotice(`${chain.name} 지갑 주소를 클립보드에 복사했습니다.`);
    } catch {
      setNotice(`브라우저가 복사를 허용하지 않았습니다. 이 전체 주소를 직접 선택해주세요: ${selectedNode.address}`);
    }
  }

  function exportReport() {
    const report = {
      product: "ChainTrace",
      demo: true,
      generatedAt: new Date().toISOString(),
      chain: chain.name,
      network: chain.network,
      rootAddress: tracedAddress,
      asset,
      filters: { direction, hops, minimum, periodDays },
      summary: {
        wallets: visibleNodeIds.size,
        transfers: graphEdges.length,
        grossTransferVolumeIncludingRehops: totalFlow,
      },
      nodes: nodes.filter((node) => visibleNodeIds.has(node.id)),
      edges: graphEdges,
      caveat: "기간 내 방향성 연결이며 소유권, 시간 인과성, FIFO 자금 귀속을 증명하지 않습니다.",
    };
    const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2)], { type: "application/json" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `chaintrace-${chain.id}-${asset.toLowerCase()}-report.json`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 1_500);
    setNotice(`${chain.name} 데모 조사 보고서를 내보냈습니다.`);
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand" aria-label="ChainTrace 홈"><span className="brand-mark" aria-hidden="true"><i /><i /><i /></span><span>ChainTrace</span><small>MULTICHAIN INTELLIGENCE</small></div>
        <div className="network-pill"><span />{chain.network}</div>
        <div className="top-status"><span className="status-dot" />Adapter catalog&nbsp; <b>{CHAINS.length} networks</b></div>
      </header>

      <section className="search-strip" aria-label="멀티체인 지갑 추적 검색">
        <form onSubmit={handleTrace} className="search-form">
          <label className="sr-only" htmlFor="chain-select">블록체인 선택</label>
          <select id="chain-select" className="chain-select" value={chainId} onChange={(event) => selectChain(event.target.value as ChainId)}>
            {CHAINS.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
          </select>
          <div className="search-icon" aria-hidden="true">⌕</div>
          <label className="sr-only" htmlFor="wallet-address">{chain.name} 지갑 주소</label>
          <input id="wallet-address" value={address} onChange={(event) => { cancelPendingTrace(); setAddress(event.target.value.trim()); setAddressError(false); setNotice(`${chain.name} 주소가 변경되었습니다. 흐름 추적을 실행하세요.`); }} placeholder={`${chain.name} 지갑 주소 입력`} autoComplete="off" spellCheck={false} aria-invalid={addressError} aria-describedby="trace-notice" />
          <button className="secondary-button" type="button" onClick={loadDemo}>데모 불러오기</button>
          <button className="trace-button" type="submit" disabled={isTracing}><span aria-hidden="true">◎</span>{isTracing ? "분석 중" : "흐름 추적"}</button>
        </form>
        <div id="trace-notice" className="notice-line" role="status"><span>DEMO</span>{notice}</div>
      </section>

      <div className="workspace">
        <aside className="filter-panel" aria-label="추적 조건">
          <div className="panel-title-row"><div><p className="eyebrow">TRACE CONTROLS</p><h2>분석 범위</h2></div><button className="icon-button" type="button" title="필터 초기화" onClick={loadDemo}>↺</button></div>
          <div className="chain-family"><span>{chain.badge}</span><div><b>{chain.name}</b><small>{chain.family.toUpperCase()} ADAPTER</small></div></div>
          <div className="control-group">
            <label>거래 방향</label>
            <div className="segmented" role="group" aria-label="거래 방향">
              {(["all", "in", "out"] as const).map((value) => <button key={value} type="button" className={direction === value ? "active" : ""} aria-pressed={direction === value} onClick={() => { cancelPendingTrace(); setDirection(value); setSelectedId("focus"); setShowAllActivities(false); }}>{value === "all" ? "전체" : value === "in" ? "입금" : "출금"}</button>)}
            </div>
          </div>
          <div className="control-group">
            <div className="label-row"><label htmlFor="hop-range">추적 단계</label><b>{hops} Hop</b></div>
            <input id="hop-range" type="range" min="1" max="3" value={hops} onChange={(event) => { cancelPendingTrace(); setHops(Number(event.target.value)); setSelectedId("focus"); setShowAllActivities(false); }} />
            <div className="range-labels"><span>1</span><span>2</span><span>3</span></div>
          </div>
          <div className="control-group">
            <label htmlFor="minimum">최소 전송액</label>
            <div className="amount-input"><input id="minimum" type="number" min="0" step="any" value={minimum} onChange={(event) => { cancelPendingTrace(); setMinimum(Number(event.target.value) || 0); setSelectedId("focus"); setShowAllActivities(false); }} /><span>{asset}</span></div>
          </div>
          <div className="control-group"><label htmlFor="asset-select">자산</label><select id="asset-select" className="select-like asset-select" value={asset} onChange={(event) => { cancelPendingTrace(); setAsset(event.target.value); setSelectedId("focus"); setShowAllActivities(false); }}>{chain.assets.map((item) => <option key={item}>{item}</option>)}</select></div>
          <div className="control-group"><label htmlFor="period-select">기간</label><select id="period-select" className="select-like asset-select" value={periodDays} onChange={(event) => { cancelPendingTrace(); setPeriodDays(Number(event.target.value)); setSelectedId("focus"); setShowAllActivities(false); }}><option value={7}>최근 7일</option><option value={30}>최근 30일</option><option value={90}>최근 90일</option></select></div>
          <div className="supported-summary"><b>{CHAINS.length}</b><span>지원 네트워크</span><small>UTXO · EVM · Solana · TRON · XRPL</small></div>
          <div className="legend"><p className="eyebrow">GRAPH LEGEND</p>{(Object.keys(KIND_LABEL) as NodeKind[]).map((kind) => <div key={kind}><span className={`legend-dot ${kind}`} />{KIND_LABEL[kind]}</div>)}</div>
          <div className="data-note">UTXO 체인의 잔돈 주소와 브리지·거래소 라벨은 추정일 수 있으며 신원을 확정하지 않습니다.</div>
        </aside>

        <section className="graph-panel" aria-label="멀티체인 지갑 자금 흐름 그래프">
          <div className="graph-header">
            <div><p className="eyebrow">{chain.name.toUpperCase()} FLOW GRAPH</p><h1>{chain.name} 지갑 자금 흐름</h1><p className="graph-caveat">기간 내 방향성 연결 · 시간 인과/FIFO 증명 아님</p></div>
            <div className="graph-metrics"><div><span>표시 지갑</span><b>{visibleNodeIds.size}</b></div><div><span>전송 건수</span><b>{graphEdges.length}</b></div><div><span>표시 전송 총액 · 중복 포함</span><b>{formatAmount(totalFlow)} <small>{asset}</small></b></div></div>
          </div>
          <div className="graph-canvas" data-testid="flow-graph">
            <div className="graph-grid" />
            <svg viewBox="0 0 1080 670" role="group" aria-label={`선택한 ${chain.name} 지갑의 입출금 흐름`} style={{ transform: `scale(${zoom})` }}>
              <defs>
                <marker id="arrow-in" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" /></marker>
                <marker id="arrow-out" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" /></marker>
                <filter id="soft-glow"><feGaussianBlur stdDeviation="4" result="blur" /><feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge></filter>
              </defs>
              {graphEdges.map((edge) => {
                const from = nodes.find((node) => node.id === edge.from)!;
                const to = nodes.find((node) => node.id === edge.to)!;
                const middleX = (from.x + to.x) / 2;
                const middleY = (from.y + to.y) / 2 - 8;
                const path = `M ${from.x} ${from.y} C ${middleX} ${from.y}, ${middleX} ${to.y}, ${to.x} ${to.y}`;
                return <g key={edge.id} className={`flow-edge ${edge.direction}`}><path className="edge-halo" d={path} /><path className="edge-line" markerEnd={`url(#arrow-${edge.direction})`} d={path} /><g className="edge-label" transform={`translate(${middleX}, ${middleY})`}><rect x="-53" y="-14" width="106" height="28" rx="8" /><text textAnchor="middle" dy="4">{formatAmount(edge.amount)} {asset}</text></g></g>;
              })}
              {nodes.filter((node) => visibleNodeIds.has(node.id)).map((node) => (
                <g key={node.id} className={`wallet-node ${node.kind} ${selectedId === node.id ? "selected" : ""}`} transform={`translate(${node.x}, ${node.y})`} role="button" tabIndex={0} aria-pressed={selectedId === node.id} aria-label={`${node.label} ${shortAddress(node.address)}`} onClick={() => { setSelectedId(node.id); setShowAllActivities(false); }} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); setSelectedId(node.id); setShowAllActivities(false); } }}>
                  <circle className="node-pulse" r={node.kind === "focus" ? 48 : 34} /><circle className="node-ring" r={node.kind === "focus" ? 35 : 25} /><circle className="node-core" r={node.kind === "focus" ? 25 : 18} filter={node.kind === "focus" ? "url(#soft-glow)" : undefined} />
                  <text className="node-icon" textAnchor="middle" dy="5">{nodeIcon(node.kind)}</text>
                  <g className="node-label" transform={`translate(0, ${node.kind === "focus" ? 58 : 44})`}><rect x="-66" y="-15" width="132" height="44" rx="9" /><text className="node-name" textAnchor="middle" y="0">{node.label}</text><text className="node-address" textAnchor="middle" y="18">{shortAddress(node.address)}</text></g>
                </g>
              ))}
            </svg>
            <div className="canvas-tools" aria-label="그래프 도구"><button type="button" title="확대" aria-label="그래프 확대" onClick={() => setZoom((value) => Math.min(1.4, Number((value + .1).toFixed(1))))}>＋</button><button type="button" title="축소" aria-label="그래프 축소" onClick={() => setZoom((value) => Math.max(.7, Number((value - .1).toFixed(1))))}>−</button><button type="button" title="화면 맞춤" aria-label="그래프 화면 맞춤" onClick={() => setZoom(1)}>⌗</button></div>
            <div className="graph-caption"><span className="pulse-dot" />확정 거래 기준 · 멀티체인 데모 스냅샷</div>
          </div>
        </section>

        <aside className="detail-panel" aria-label="선택한 지갑 상세정보">
          <div className="detail-top"><p className="eyebrow">WALLET INSPECTOR</p><span className={`type-badge ${selectedNode.kind}`}>{selectedNode.tag}</span></div>
          <div className={`detail-avatar ${selectedNode.kind}`}>{nodeIcon(selectedNode.kind)}</div>
          <h2>{selectedNode.label}</h2>
          <button className="address-copy" type="button" onClick={copyAddress} title="주소 복사" aria-label={`${selectedNode.label} 주소 복사`}><span>{shortAddress(selectedNode.address)}</span><b aria-hidden="true">□</b></button>
          <div className="balance-card"><span>현재 {asset} 잔액</span><strong>{selectedNode.balance}</strong><small>{chain.network}</small></div>
          <dl className="wallet-facts"><div><dt>최초 확인</dt><dd>{selectedNode.firstSeen}</dd></div><div><dt>전체 거래</dt><dd>{formatAmount(selectedNode.transactions)}건</dd></div><div><dt>예시 위험 점수</dt><dd><span className="risk-level">데모 · 42</span></dd></div></dl>
          <div className="analyst-note"><span>분석 메모</span><p>{selectedNode.note}</p></div>
          <div className="activity-header"><div><p className="eyebrow">RECENT ACTIVITY</p><h3>관련 거래</h3></div>{selectedActivities.length > 4 && <button type="button" onClick={() => setShowAllActivities((value) => !value)}>{showAllActivities ? "간단히" : "전체 보기"}</button>}</div>
          <div className="activity-list">
            {selectedActivities.slice(0, showAllActivities ? selectedActivities.length : 4).map((edge) => {
              const outgoing = edge.from === selectedNode.id;
              const counterpartId = outgoing ? edge.to : edge.from;
              return <button key={edge.id} type="button" className="activity-item" onClick={() => { setSelectedId(counterpartId); setShowAllActivities(false); }}><span className={outgoing ? "out" : "in"}>{outgoing ? "↗" : "↙"}</span><div><b>{outgoing ? "출금" : "입금"} · {edge.tx}</b><small>{edge.time}</small></div><strong className={outgoing ? "negative" : "positive"}>{outgoing ? "−" : "+"}{formatAmount(edge.amount)}</strong></button>;
            })}
          </div>
          <button className="report-button" type="button" onClick={exportReport}>조사 보고서 내보내기 <span aria-hidden="true">↗</span></button>
        </aside>
      </div>
    </main>
  );
}
