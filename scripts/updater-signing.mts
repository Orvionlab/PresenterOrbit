import { createHash, createPrivateKey, createPublicKey, randomBytes, scryptSync, sign } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

// Writes the minisign signature that `tauri signer sign` wrote, so installed Tauri apps and the GPUI updater both
// verify it: a prehashed ("ED", BLAKE2b-512) Ed25519 signature, base64 of the whole signature file text.
// The secret key is the Tauri/rsign form: base64 of the key file, scrypt-encrypted with the (possibly empty) password.

const ed25519Pkcs8Prefix = Buffer.from("302e020100300506032b657004220420", "hex");
const scryptMemoryLimitMax = 1 << 30;
const scryptLog2Max = 20;

/** The scrypt cost parameters minisign derives from the limits stored in the key (minisign `raw_scrypt_params`). */
function scryptParameters(memoryLimit: number, opsLimit: bigint) {
  const ops = opsLimit > 32768n ? opsLimit : 32768n;
  const r = 8n;
  let log2 = 1;
  let p: bigint;
  if (ops < BigInt(Math.floor(memoryLimit / 32))) {
    p = 1n;
    const maxN = ops / (r * 4n);
    while (log2 < 63 && 1n << BigInt(log2) <= maxN / 2n) log2 += 1;
  } else {
    const maxN = BigInt(memoryLimit) / (r * 128n);
    while (log2 < 63 && 1n << BigInt(log2) <= maxN / 2n) log2 += 1;
    const maxRp = ops / 4n / (1n << BigInt(log2));
    p = (maxRp < 0x3fffffffn ? maxRp : 0x3fffffffn) / r;
  }
  if (log2 > scryptLog2Max) throw new Error("The updater signing key asks for more scrypt memory than allowed.");
  return { N: 2 ** log2, r: Number(r), p: Number(p) };
}

/** Decrypts the secret key text into its 32-byte Ed25519 seed and 8-byte key id. */
function decryptSecretKey(keyText: string, password: string) {
  const lines = keyText.split(/\r?\n/).filter((line) => line.trim());
  const body = Buffer.from(lines[1] ?? "", "base64");
  if (body.length !== 158 || body.toString("latin1", 0, 2) !== "Ed" || body.toString("latin1", 2, 4) !== "Sc")
    throw new Error("TAURI_SIGNING_PRIVATE_KEY is not a password-protected minisign secret key.");
  const salt = body.subarray(6, 38);
  const opsLimit = body.readBigUInt64LE(38);
  const memoryLimit = Number(body.readBigUInt64LE(46));
  if (memoryLimit > scryptMemoryLimitMax) throw new Error("The updater signing key asks for too much scrypt memory.");
  const { N, r, p } = scryptParameters(memoryLimit, opsLimit);
  const stream = scryptSync(password, salt, 104, { N, r, p, maxmem: 256 * N * r + (1 << 24) });
  const plain = Buffer.alloc(104);
  for (let index = 0; index < 104; index += 1) plain[index] = body[54 + index] ^ stream[index];
  const keyId = plain.subarray(0, 8);
  const seed = plain.subarray(8, 40);
  const publicKey = plain.subarray(40, 72);
  const derived = createPublicKey(privateKeyFromSeed(seed)).export({ format: "der", type: "spki" }).subarray(-32);
  if (!derived.equals(publicKey))
    throw new Error("The updater signing key could not be decrypted; check the password.");
  return { keyId: Buffer.from(keyId), seed: Buffer.from(seed) };
}

function privateKeyFromSeed(seed: Buffer) {
  return createPrivateKey({ key: Buffer.concat([ed25519Pkcs8Prefix, seed]), format: "der", type: "pkcs8" });
}

/** The `.sig` text for `data`: the manifest `signature` value (base64 of the minisign signature file). */
export function signUpdaterData(
  data: Buffer,
  fileName: string,
  privateKey: string,
  password: string,
  now = Date.now(),
) {
  const keyText = existsSync(privateKey)
    ? readFileSync(privateKey, "utf8")
    : Buffer.from(privateKey, "base64").toString("utf8");
  const { keyId, seed } = decryptSecretKey(keyText, password);
  const key = privateKeyFromSeed(seed);
  const signature = sign(null, createHash("blake2b512").update(data).digest(), key);
  const trustedComment = `timestamp:${Math.floor(now / 1000)}\tfile:${fileName}`;
  const global = sign(null, Buffer.concat([signature, Buffer.from(trustedComment)]), key);
  const raw = Buffer.concat([Buffer.from("ED"), keyId, signature]);
  const text =
    `untrusted comment: signature from tauri secret key\n${raw.toString("base64")}\n` +
    `trusted comment: ${trustedComment}\n${global.toString("base64")}\n`;
  return Buffer.from(text).toString("base64");
}

/** Writes `<artifact>.sig` using `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. */
export function signUpdaterArtifact(artifact: string, env: NodeJS.ProcessEnv = process.env) {
  const privateKey = env.TAURI_SIGNING_PRIVATE_KEY;
  if (!privateKey) throw new Error("TAURI_SIGNING_PRIVATE_KEY is not set.");
  const signature = signUpdaterData(
    readFileSync(artifact),
    basename(artifact),
    privateKey,
    env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  );
  writeFileSync(`${artifact}.sig`, signature);
  return signature;
}

/** Test helper: a throwaway minisign key pair in the Tauri encoding (cheap scrypt limits). */
export function generateThrowawayKey(password: string) {
  const seed = randomBytes(32);
  const publicKey = createPublicKey(privateKeyFromSeed(seed)).export({ format: "der", type: "spki" }).subarray(-32);
  const keyId = randomBytes(8);
  const salt = randomBytes(32);
  const opsLimit = 32768n;
  const memoryLimit = 1 << 20;
  const { N, r, p } = scryptParameters(memoryLimit, opsLimit);
  const stream = scryptSync(password, salt, 104, { N, r, p, maxmem: 256 * N * r + (1 << 24) });
  const plain = Buffer.concat([keyId, seed, publicKey, Buffer.alloc(32)]);
  const header = Buffer.alloc(54);
  header.write("EdScB2", 0, "latin1");
  salt.copy(header, 6);
  header.writeBigUInt64LE(opsLimit, 38);
  header.writeBigUInt64LE(BigInt(memoryLimit), 46);
  const encrypted = Buffer.from(plain.map((byte, index) => byte ^ stream[index]));
  const secret = `untrusted comment: rsign encrypted secret key\n${Buffer.concat([header, encrypted]).toString("base64")}\n`;
  const publicText = `untrusted comment: minisign public key: throwaway\n${Buffer.concat([Buffer.from("Ed"), keyId, publicKey]).toString("base64")}\n`;
  return {
    privateKey: Buffer.from(secret).toString("base64"),
    publicKey: Buffer.from(publicText).toString("base64"),
  };
}
