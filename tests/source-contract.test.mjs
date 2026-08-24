import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("keeps the investigation controls and accessible graph contract", async () => {
  const source = await readFile(new URL("../app/page.tsx", import.meta.url), "utf8");
  for (const expected of [
    'data-testid="flow-graph"',
    'aria-label="멀티체인 지갑 자금 흐름 그래프"',
    'id="chain-select"',
    'Bitcoin',
    'Ethereum',
    'Solana',
    'Dogecoin',
    'XRP Ledger',
    'role="status"',
    'type="range"',
    'id="period-select"',
    'onClick={exportReport}',
    'aria-label="그래프 확대"',
  ]) {
    assert.match(source, new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  }
  for (const chain of [
    "Bitcoin", "Ethereum", "Ethereum Classic", "Solana", "TRON", "BNB Chain", "Polygon",
    "Arbitrum", "Optimism", "Base", "Avalanche", "Dogecoin", "Litecoin", "XRP Ledger",
  ]) {
    assert.match(source, new RegExp(`name: "${chain.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}"`));
  }
  assert.equal([...source.matchAll(/\{ id: "[^"]+", name:/g)].length, 14);
  for (const behavior of ["minHops", "ageDays", "setTracedAddress", "aria-invalid", "aria-pressed", "전체 주소를 직접 선택해주세요"]) {
    assert.match(source, new RegExp(behavior));
  }
  const styles = await readFile(new URL("../app/globals.css", import.meta.url), "utf8");
  assert.match(styles, /prefers-reduced-motion/);
  assert.match(styles, /color-scheme:\s*light/);
  assert.match(styles, /--bg:\s*#f3f7fb/i);
  assert.doesNotMatch(styles, /--bg:\s*#07111f/i);
  assert.doesNotMatch(styles, /#07111f|#091421|#081320|#081522|#0a1726|#0d1c2d/i);
  assert.match(styles, /select:focus-visible/);
  assert.match(styles, /overflow-x:\s*auto/);
  assert.match(styles, /graph-canvas svg \{ width:\s*760px; min-width:\s*760px;/);
});
