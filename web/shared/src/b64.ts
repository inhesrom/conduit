/** UTF-8-safe base64 helpers. `atob`/`btoa` alone mangle multi-byte text, and
 * terminal bytes must round-trip exactly — never through JS strings. */

export function b64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

export function bytesToB64(bytes: Uint8Array): string {
  let bin = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  }
  return btoa(bin);
}

export function textToB64(text: string): string {
  return bytesToB64(new TextEncoder().encode(text));
}
