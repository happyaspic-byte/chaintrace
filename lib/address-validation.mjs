const BITCOIN_BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const RIPPLE_BASE58 = "rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz";
const BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

function decodeBase58(value, alphabet = BITCOIN_BASE58) {
  if (!value || value.length > 128) return null;
  let number = 0n;
  for (const character of value) {
    const digit = alphabet.indexOf(character);
    if (digit < 0) return null;
    number = number * 58n + BigInt(digit);
  }
  const bytes = [];
  while (number > 0n) {
    bytes.unshift(Number(number & 255n));
    number >>= 8n;
  }
  let leadingZeroes = 0;
  while (value[leadingZeroes] === alphabet[0]) leadingZeroes += 1;
  return Uint8Array.from([...new Array(leadingZeroes).fill(0), ...bytes]);
}

function rotateRight(value, bits) {
  return (value >>> bits) | (value << (32 - bits));
}

function sha256(input) {
  const constants = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
  ];
  const initial = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ];
  const paddedLength = Math.ceil((input.length + 9) / 64) * 64;
  const padded = new Uint8Array(paddedLength);
  padded.set(input);
  padded[input.length] = 0x80;
  new DataView(padded.buffer).setUint32(paddedLength - 4, input.length * 8, false);
  const hash = [...initial];
  const schedule = new Uint32Array(64);
  const view = new DataView(padded.buffer);

  for (let offset = 0; offset < paddedLength; offset += 64) {
    for (let index = 0; index < 16; index += 1) {
      schedule[index] = view.getUint32(offset + index * 4, false);
    }
    for (let index = 16; index < 64; index += 1) {
      const left = schedule[index - 15];
      const right = schedule[index - 2];
      const sigma0 = rotateRight(left, 7) ^ rotateRight(left, 18) ^ (left >>> 3);
      const sigma1 = rotateRight(right, 17) ^ rotateRight(right, 19) ^ (right >>> 10);
      schedule[index] = (schedule[index - 16] + sigma0 + schedule[index - 7] + sigma1) >>> 0;
    }

    let [a, b, c, d, e, f, g, h] = hash;
    for (let index = 0; index < 64; index += 1) {
      const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const choose = (e & f) ^ (~e & g);
      const temporary1 = (h + sum1 + choose + constants[index] + schedule[index]) >>> 0;
      const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temporary2 = (sum0 + majority) >>> 0;
      h = g;
      g = f;
      f = e;
      e = (d + temporary1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temporary1 + temporary2) >>> 0;
    }
    [a, b, c, d, e, f, g, h].forEach((value, index) => {
      hash[index] = (hash[index] + value) >>> 0;
    });
  }

  const output = new Uint8Array(32);
  const outputView = new DataView(output.buffer);
  hash.forEach((value, index) => outputView.setUint32(index * 4, value, false));
  return output;
}

function doubleSha256(value) {
  return sha256(sha256(value));
}

async function validatesBase58Check(value, alphabet, validPrefixes) {
  const decoded = decodeBase58(value, alphabet);
  if (!decoded || decoded.length !== 25 || !validPrefixes.includes(decoded[0])) return false;
  const checksum = doubleSha256(decoded.slice(0, 21));
  return decoded.slice(21).every((byte, index) => byte === checksum[index]);
}

function bech32Polymod(values) {
  const generators = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
  let checksum = 1;
  for (const value of values) {
    const top = checksum >>> 25;
    checksum = (((checksum & 0x1ffffff) << 5) ^ value) >>> 0;
    generators.forEach((generator, index) => {
      if (((top >>> index) & 1) !== 0) checksum = (checksum ^ generator) >>> 0;
    });
  }
  return checksum >>> 0;
}

function convertBits5To8(values) {
  let accumulator = 0;
  let bits = 0;
  const output = [];
  for (const value of values) {
    if (value > 31) return null;
    accumulator = ((accumulator << 5) | value) & 4095;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      output.push((accumulator >>> bits) & 255);
    }
  }
  if (bits >= 5 || ((accumulator << (8 - bits)) & 255) !== 0) return null;
  return output;
}

function validatesSegwit(value, expectedHrp) {
  if (value.length < 14 || value.length > 90) return false;
  const hasLower = /[a-z]/.test(value);
  const hasUpper = /[A-Z]/.test(value);
  if (hasLower && hasUpper) return false;
  const normalized = value.toLowerCase();
  const separator = normalized.lastIndexOf("1");
  if (separator < 1 || normalized.slice(0, separator) !== expectedHrp || separator + 7 > normalized.length) return false;
  const values = [];
  for (const character of normalized.slice(separator + 1)) {
    const index = BECH32_CHARSET.indexOf(character);
    if (index < 0) return false;
    values.push(index);
  }
  if (values.length < 7 || values[0] > 16) return false;
  const program = convertBits5To8(values.slice(1, -6));
  if (!program || program.length < 2 || program.length > 40) return false;
  if (values[0] === 0 && program.length !== 20 && program.length !== 32) return false;
  const expandedHrp = [
    ...[...expectedHrp].map((character) => character.charCodeAt(0) >>> 5),
    0,
    ...[...expectedHrp].map((character) => character.charCodeAt(0) & 31),
  ];
  const polymod = bech32Polymod([...expandedHrp, ...values]);
  return values[0] === 0 ? polymod === 1 : polymod === 0x2bc830a3;
}

export async function isValidAddress(family, chainId, input) {
  if (input.length > 128) return false;
  const value = input.trim();
  if (family === "evm") return /^0x[0-9a-fA-F]{40}$/.test(value);
  if (family === "solana") return decodeBase58(value)?.length === 32;
  if (family === "tron") return validatesBase58Check(value, BITCOIN_BASE58, [0x41]);
  if (family === "xrpl") return validatesBase58Check(value, RIPPLE_BASE58, [0]);
  if (chainId === "bitcoin" && value.toLowerCase().startsWith("bc1")) return validatesSegwit(value, "bc");
  if (chainId === "litecoin" && value.toLowerCase().startsWith("ltc1")) return validatesSegwit(value, "ltc");
  if (chainId === "bitcoin") return validatesBase58Check(value, BITCOIN_BASE58, [0x00, 0x05]);
  if (chainId === "dogecoin") return validatesBase58Check(value, BITCOIN_BASE58, [0x1e, 0x16]);
  if (chainId === "litecoin") return validatesBase58Check(value, BITCOIN_BASE58, [0x30, 0x32, 0x05]);
  return false;
}
