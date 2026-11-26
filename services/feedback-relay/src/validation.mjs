import { HttpError, MAX_IMAGE_BYTES, MAX_CAPTURE_BYTES, MAX_DIAGNOSTICS_BYTES, encoder, sha256 } from "./security.mjs";

const categories = new Set(["device", "battery", "dpi", "other"]);
const families = new Set(["mouse", "keyboard", "headset", "gamepad", "other"]);
const mimes = new Set(["image/png", "image/jpeg", "image/webp"]);
const captureApis = new Set(["write", "input", "input-timeout", "get-feature", "get-input", "set-output", "set-feature"]);

function invalid(message) { throw new HttpError(400, "validation", message); }

function object(value, name, allowed) {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid(`${name} must be an object.`);
  for (const key of Object.keys(value)) if (!allowed.includes(key)) invalid(`${name} contains an unknown field.`);
}

function text(value, name, minimum, maximum, multiline = false) {
  if (typeof value !== "string" || value.length > maximum * 2) invalid(`${name} has an invalid length.`);
  const result = value.trim();
  const length = Array.from(result).length;
  if (length < minimum || length > maximum || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(result)
    || (!multiline && /[\r\n\t]/u.test(result))) invalid(`${name} has an invalid length or contains control characters.`);
  return result;
}

function uuid(value) {
  if (typeof value !== "string" || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value)) {
    invalid("requestId must be a UUID.");
  }
  return value.toLowerCase();
}

function dimensions(width, height) {
  if (width < 1 || height < 1 || width > 16384 || height > 16384 || width * height > 60_000_000) {
    invalid("Image dimensions are unsupported.");
  }
}

function signature(bytes, mime) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (mime === "image/png") {
    if (bytes.length < 33 || [137, 80, 78, 71, 13, 10, 26, 10].some((byte, index) => bytes[index] !== byte)
      || view.getUint32(8) !== 13 || view.getUint32(12) !== 0x49484452) invalid("Invalid PNG image.");
    dimensions(view.getUint32(16), view.getUint32(20));
    return;
  }
  if (mime === "image/jpeg") {
    if (bytes.length < 12 || bytes[0] !== 0xff || bytes[1] !== 0xd8) invalid("Invalid JPEG image.");
    let offset = 2;
    while (offset + 4 <= bytes.length) {
      if (bytes[offset++] !== 0xff) invalid("Invalid JPEG image.");
      while (bytes[offset] === 0xff) offset++;
      const marker = bytes[offset++];
      if (marker === 0xd9 || marker === 0xda) break;
      if (marker === 1 || (marker >= 0xd0 && marker <= 0xd7)) continue;
      if (offset + 2 > bytes.length) break;
      const length = view.getUint16(offset);
      if (length < 2 || offset + length > bytes.length) break;
      if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker)) {
        if (length < 8) break;
        dimensions(view.getUint16(offset + 5), view.getUint16(offset + 3));
        return;
      }
      offset += length;
    }
    invalid("JPEG frame dimensions are missing.");
  }
  if (bytes.length < 25 || view.getUint32(0) !== 0x52494646 || view.getUint32(8) !== 0x57454250
    || view.getUint32(4, true) !== bytes.length - 8) invalid("Invalid WebP image.");
  const tag = view.getUint32(12);
  if (tag === 0x56503858 && bytes.length >= 30) {
    dimensions(1 + bytes[24] + (bytes[25] << 8) + (bytes[26] << 16),
      1 + bytes[27] + (bytes[28] << 8) + (bytes[29] << 16));
  } else if (tag === 0x5650384c && bytes[20] === 0x2f) {
    dimensions(1 + ((bytes[22] & 0x3f) << 8) + bytes[21],
      1 + ((bytes[24] & 0x0f) << 10) + (bytes[23] << 2) + (bytes[22] >> 6));
  } else if (tag === 0x56503820 && bytes.length >= 30 && bytes[23] === 0x9d
    && bytes[24] === 1 && bytes[25] === 0x2a) {
    dimensions(view.getUint16(26, true) & 0x3fff, view.getUint16(28, true) & 0x3fff);
  } else invalid("Unsupported WebP frame.");
}

async function image(value) {
  object(value, "image", ["name", "mimeType", "dataBase64"]);
  const name = text(value.name, "image.name", 1, 160);
  if (/[\\/]/.test(name) || name === "." || name === "..") invalid("Image names cannot contain paths.");
  if (!mimes.has(value.mimeType)) invalid("Only PNG, JPEG and WebP images are accepted.");
  const encoded = value.dataBase64;
  if (typeof encoded !== "string" || encoded.length < 4 || encoded.length > 4 * Math.ceil(MAX_IMAGE_BYTES / 3)
    || encoded.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) invalid("Invalid or oversized base64 image.");
  let raw;
  try { raw = atob(encoded); } catch { invalid("Invalid base64 image."); }
  if (raw.length > MAX_IMAGE_BYTES || btoa(raw) !== encoded) invalid("Invalid or oversized base64 image.");
  // Avoid Uint8Array.from(string): its iterable conversion can materialize a
  // multi-million-element temporary array for a permitted 5 MiB image.
  const bytes = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index++) bytes[index] = raw.charCodeAt(index);
  signature(bytes, value.mimeType);
  return { name, mimeType: value.mimeType, bytes, digest: await sha256(bytes) };
}

function integer(value, name, maximum = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) invalid(`${name} must be an unsigned integer in range.`);
}

async function capture(value) {
  object(value, "capture", ["name", "mimeType", "dataBase64"]);
  if (value.name !== "raw.jsonl" || value.mimeType !== "application/x-ndjson") invalid("Capture must be raw.jsonl with application/x-ndjson MIME type.");
  const encoded = value.dataBase64;
  if (typeof encoded !== "string" || encoded.length < 4 || encoded.length > 4 * Math.ceil(MAX_CAPTURE_BYTES / 3)
    || encoded.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(encoded)) invalid("Invalid or oversized base64 capture.");
  let raw;
  try { raw = atob(encoded); } catch { invalid("Invalid base64 capture."); }
  if (raw.length > MAX_CAPTURE_BYTES || btoa(raw) !== encoded) invalid("Invalid or oversized base64 capture.");
  const bytes = new Uint8Array(raw.length);
  for (let index = 0; index < raw.length; index++) bytes[index] = raw.charCodeAt(index);
  let source;
  try { source = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes); }
  catch { invalid("Capture must contain valid UTF-8 JSONL."); }
  const lines = source.split(/\r?\n/);
  if (lines.at(-1) === "") lines.pop();
  if (lines.length < 3 || lines.length > 1026) invalid("Capture must contain a header, at most 1024 records and a final end record.");
  let records;
  try { records = lines.map(line => JSON.parse(line)); }
  catch { invalid("Capture must contain valid UTF-8 JSONL without blank lines."); }
  const header = records[0];
  object(header, "capture header", ["schema", "app", "started_unix_ms", "vendor_id", "product_id", "usage_page",
    "usage", "input_bytes", "output_bytes", "feature_bytes", "scope", "privacy"]);
  if (header.schema !== 1) invalid("Capture schema must be 1.");
  text(header.app, "capture.app", 1, 80);
  text(header.scope, "capture.scope", 1, 512);
  text(header.privacy, "capture.privacy", 1, 1024);
  integer(header.started_unix_ms, "capture.started_unix_ms");
  for (const field of ["vendor_id", "product_id", "usage_page", "usage", "input_bytes", "output_bytes", "feature_bytes"]) {
    integer(header[field], `capture.${field}`, 65535);
  }
  let transfers = 0;
  let steps = 0;
  let hasData = false;
  for (const record of records.slice(1, -1)) {
    if (record?.kind === "transfer") {
      object(record, "capture transfer", ["kind", "elapsed_ms", "api", "success", "hex"]);
      integer(record.elapsed_ms, "capture.elapsed_ms");
      if (!captureApis.has(record.api) || typeof record.success !== "boolean"
        || typeof record.hex !== "string" || record.hex.length > 8192 || !/^(?:[0-9a-fA-F]{2})*$/.test(record.hex)) {
        invalid("Invalid capture transfer.");
      }
      transfers++;
      hasData ||= record.hex.length > 0;
    } else if (record?.kind === "step") {
      object(record, "capture step", ["kind", "elapsed_ms", "step", "label"]);
      integer(record.elapsed_ms, "capture.elapsed_ms");
      integer(record.step, "capture.step", 4);
      text(record.label, "capture.label", 1, 120);
      steps++;
    } else invalid("Unknown capture record kind.");
  }
  const end = records.at(-1);
  object(end, "capture end", ["kind", "reason", "transfers"]);
  if (end.kind !== "end" || end.transfers !== transfers) invalid("Capture must end with the matching transfer count.");
  text(end.reason, "capture.reason", 1, 80);
  if (!hasData) invalid("Capture must contain at least one transfer with nonempty data.");
  return { name: value.name, mimeType: value.mimeType, bytes, digest: await sha256(bytes), transfers, steps };
}

export async function feedback(value) {
  object(value, "feedback", ["schema", "requestId", "replyKey", "category", "device", "description", "diagnostics", "images", "appVersion", "capture"]);
  if (value.schema !== 1) invalid("schema must be 1.");
  const requestId = uuid(value.requestId);
  if (typeof value.replyKey !== "string" || !/^[a-f0-9]{64}$/i.test(value.replyKey)) invalid("replyKey must contain 32 random bytes encoded as hexadecimal.");
  if (!categories.has(value.category)) invalid("Unknown feedback category.");
  object(value.device, "device", ["name", "family", "connection", "brand", "model"]);
  if (!families.has(value.device.family)) invalid("Unknown device family.");
  const device = {
    name: text(value.device.name, "device.name", 1, 120),
    family: value.device.family,
    connection: text(value.device.connection, "device.connection", 1, 80),
  };
  if (value.device.brand !== undefined) device.brand = text(value.device.brand, "device.brand", 1, 80);
  if (value.device.model !== undefined) device.model = text(value.device.model, "device.model", 1, 120);
  const description = text(value.description, "description", 5, 2000, true);
  let diagnostics;
  if (value.diagnostics !== undefined) {
    diagnostics = text(value.diagnostics, "diagnostics", 0, MAX_DIAGNOSTICS_BYTES, true);
    if (encoder.encode(diagnostics).byteLength > MAX_DIAGNOSTICS_BYTES) invalid("diagnostics exceeds 20 KiB.");
  }
  let appVersion;
  if (value.appVersion !== undefined) {
    appVersion = text(value.appVersion, "appVersion", 1, 40);
    if (!/^[0-9A-Za-z.+_-]+$/.test(appVersion)) invalid("Invalid appVersion.");
  }
  if (value.images !== undefined && (!Array.isArray(value.images) || value.images.length > 3)) invalid("At most three images are allowed.");
  const images = [];
  for (const item of value.images ?? []) images.push(await image(item));
  const attachment = value.capture === undefined ? undefined : await capture(value.capture);
  const replyKey = value.replyKey.toLowerCase();
  const canonical = { schema: 1, requestId, category: value.category, device, description,
    appVersion: appVersion ?? null, diagnostics: diagnostics ?? null,
    images: images.map(({ name, mimeType, digest }) => ({ name, mimeType, digest })) };
  if (attachment) canonical.capture = { name: attachment.name, mimeType: attachment.mimeType, digest: attachment.digest };
  return { ...canonical, replyKey, images, ...(attachment ? { capture: attachment } : {}),
    fingerprint: await sha256(JSON.stringify(canonical)) };
}

export async function reply(value) {
  object(value, "reply", ["schema", "requestId", "body"]);
  if (value.schema !== 1) invalid("schema must be 1.");
  const requestId = uuid(value.requestId);
  const body = text(value.body, "body", 1, 2000, true);
  return { requestId, body, fingerprint: await sha256(body) };
}
