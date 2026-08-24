import assert from "node:assert/strict";
import test from "node:test";

import { isValidAddress } from "../lib/address-validation.mjs";

test("validates checksums and byte lengths across every supported chain family", async () => {
  const valid = [
    ["utxo", "bitcoin", "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQSC"],
    ["utxo", "bitcoin", "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"],
    ["utxo", "dogecoin", "DCJZ1q8JsQTZQUwu12Yv5KZzckfaaZ4LnY"],
    ["utxo", "litecoin", "LWrzZrgBvieg6SFK2GbwtZDDAy8hsHoMLL"],
    ["evm", "ethereum", "0xb0e3201a3c1cefb260e007781f36fbe5bc1bf624"],
    ["evm", "ethereum-classic", "0xb794f5ea0ba39494ce839613fffba74279579268"],
    ["solana", "solana", "B7eKLu5DFf5bafEKpZPyBFuoBmMZtuworcbqDSaoPPDP"],
    ["tron", "tron", "TMKhcf7v9tt1Tnkxrrbr6CNZHMiwxrsgAd"],
    ["xrpl", "xrp", "rGzDj59ipb1tQjoWmJ2DsLDypuaZ6f9EhC"],
  ];
  for (const [family, chain, address] of valid) {
    assert.equal(await isValidAddress(family, chain, address), true, `${chain}: ${address}`);
  }
});

test("rejects checksum mutations, wrong byte lengths, and oversized inputs", async () => {
  const invalid = [
    ["utxo", "bitcoin", "1NXpPiMGYE6w3pEdCUiQ1xU9WNAYR9GQS1"],
    ["utxo", "dogecoin", `D${"1".repeat(33)}`],
    ["utxo", "litecoin", "LWrzZrgBvieg6SFK2GbwtZDDAy8hsHoML1"],
    ["solana", "solana", "1111111111111111111111111111111"],
    ["tron", "tron", "TMKhcf7v9tt1Tnkxrrbr6CNZHMiwxrsgA1"],
    ["xrpl", "xrp", "rGzDj59ipb1tQjoWmJ2DsLDypuaZ6f9Eh1"],
    ["evm", "ethereum", `0x${"1".repeat(129)}`],
  ];
  for (const [family, chain, address] of invalid) {
    assert.equal(await isValidAddress(family, chain, address), false, `${chain}: ${address}`);
  }
});
