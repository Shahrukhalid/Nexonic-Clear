import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { exists, readFile, writeFile } from "@tauri-apps/plugin-fs";
import "./style.css";

type Kind = "exif" | "xmp" | "iptc" | "icc" | "c2pa" | "comment" | "other";
type Format = "jpeg" | "png" | "webp";

interface Block { kind: Kind; label: string; size: number }
interface Credentials {
  present: boolean; state: string; claim_generator: string | null; title: string | null;
  issuer: string | null; signed_at: string | null; algorithm: string | null;
  ai_generated: boolean; actions: string[]; manifests: number; problems: string[];
}
interface ExifSummary { fields: [string, string][]; has_gps: boolean; has_thumbnail: boolean; tag_count: number }
interface Inspection {
  format: Format; width: number; height: number; has_alpha: boolean; animated: boolean; size: number;
  blocks: Block[]; exif: ExifSummary | null; credentials: Credentials; image_fingerprint: string;
}
interface CleanReport {
  before: Inspection; after: Inspection; image_unchanged: boolean; still_present: Kind[];
  bytes_removed: number; notes: string[];
}
interface Options {
  exif: boolean; xmp: boolean; iptc: boolean; icc: boolean; c2pa: boolean;
  comments: boolean; other: boolean; keep_orientation: boolean;
}
interface Item {
  id: number; name: string; bytes: Uint8Array; url: string;
  info?: Inspection; error?: string; report?: CleanReport; outputId?: number; outputUrl?: string;
  saved?: string;
}

const KIND_LABEL: Record<Kind, string> = {
  exif: "EXIF", xmp: "XMP", iptc: "IPTC", icc: "Colour profile", c2pa: "Content Credentials",
  comment: "Comments and text", other: "Other data",
};
const MIME: Record<Format, string> = { jpeg: "image/jpeg", png: "image/png", webp: "image/webp" };
const EXT: Record<Format, string> = { jpeg: "jpg", png: "png", webp: "webp" };
const SENSITIVE = new Set(["Location", "Owner", "Camera serial", "Lens serial", "Artist"]);

const TOGGLES: { key: keyof Options; title: string; hint: string }[] = [
  { key: "exif", title: "EXIF", hint: "Camera, dates, GPS location, serial numbers" },
  { key: "xmp", title: "XMP", hint: "Editing history, descriptions, AI labels written as XMP" },
  { key: "iptc", title: "IPTC", hint: "Captions, credits, keywords" },
  { key: "comments", title: "Comments and text", hint: "Comments and text fields" },
  { key: "other", title: "Other data", hint: "Maker data, unknown blocks, data after the image" },
  { key: "icc", title: "Colour profile", hint: "Keep it for accurate colours" },
  { key: "c2pa", title: "Content Credentials", hint: "Signed C2PA provenance record" },
  { key: "keep_orientation", title: "Keep orientation", hint: "Rotated photos stay upright" },
];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const isAndroid = /Android/i.test(navigator.userAgent);
const items: Item[] = [];
let selected = -1;
let nextId = 1;
let busy = false;
const options: Options = {
  exif: true, xmp: true, iptc: true, icc: false, c2pa: false, comments: true, other: true, keep_orientation: true,
};

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}
function size(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}
function sumKind(info: Inspection, kind: Kind): number {
  return info.blocks.filter((b) => b.kind === kind).reduce((a, b) => a + b.size, 0);
}

// ---------- IPC ----------
// Desktop returns an ArrayBuffer; the Android bridge may return a plain array.
const toBytes = (v: ArrayBuffer | number[] | null | undefined) =>
  v instanceof ArrayBuffer ? new Uint8Array(v) : new Uint8Array(v ?? []);
const api = {
  inspect: (bytes: Uint8Array) => invoke<Inspection>("inspect", bytes),
  clean: async (bytes: Uint8Array, o: Options) => {
    await invoke("set_options", { options: o });
    return invoke<{ id: number; report: CleanReport }>("clean", bytes);
  },
  output: async (id: number) => toBytes(await invoke<ArrayBuffer | number[]>("output", { id })),
  discard: (ids: number[]) => invoke("discard", { ids }),
};

// ---------- Adding files ----------
function baseName(path: string): string {
  let name = path.split(/[\\/]/).pop() || "image";
  try { name = decodeURIComponent(name); } catch { /* keep raw */ }
  return name.split(/[\\/:]/).pop() || "image";
}

async function addFiles(files: { name: string; bytes: Uint8Array }[]) {
  for (const f of files) {
    const item: Item = { id: nextId++, name: f.name, bytes: f.bytes, url: "" };
    try {
      item.info = await api.inspect(f.bytes);
      item.url = URL.createObjectURL(new Blob([f.bytes as BlobPart], { type: MIME[item.info.format] }));
      if (!/\.[a-z0-9]{3,4}$/i.test(item.name)) item.name += "." + EXT[item.info.format];
    } catch (e) {
      item.error = String(e);
    }
    items.push(item);
    if (selected < 0) selected = 0;
  }
  render();
}

async function pickFiles() {
  try {
    const picked = await open({
      multiple: true,
      filters: [{ name: "Images", extensions: ["jpg", "jpeg", "png", "webp"] }],
    });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    const files = [];
    for (const p of paths) files.push({ name: baseName(p), bytes: await readFile(p) });
    await addFiles(files);
  } catch (e) {
    status(`Could not open the files: ${e}`, true);
  }
}

async function onDrop(ev: DragEvent) {
  ev.preventDefault();
  document.body.classList.remove("dragging");
  const list = Array.from(ev.dataTransfer?.files ?? []);
  const files = await Promise.all(list.map(async (f) => ({ name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) })));
  if (files.length) await addFiles(files);
}

// ---------- Cleaning and saving ----------
async function runClean() {
  if (busy) return;
  const todo = items.filter((i) => i.info);
  if (!todo.length) return;
  busy = true;
  render();
  let done = 0;
  const old = todo.map((i) => i.outputId).filter((x): x is number => x !== undefined);
  if (old.length) await api.discard(old);
  for (const item of todo) {
    status(`Cleaning ${++done} of ${todo.length}…`);
    item.report = undefined;
    item.saved = undefined;
    if (item.outputUrl) URL.revokeObjectURL(item.outputUrl);
    try {
      const r = await api.clean(item.bytes, options);
      item.outputId = r.id;
      item.report = r.report;
      const out = await api.output(r.id);
      item.outputUrl = URL.createObjectURL(new Blob([out as BlobPart], { type: MIME[r.report.after.format] }));
    } catch (e) {
      item.error = String(e);
    }
  }
  busy = false;
  const failed = todo.filter((i) => !i.report || !i.report.image_unchanged || i.report.still_present.length).length;
  status(failed ? `${todo.length - failed} cleaned and verified, ${failed} need attention.` : `${todo.length} cleaned and verified. Save to keep the results.`, failed > 0);
  render();
}

function outName(item: Item): string {
  const dot = item.name.lastIndexOf(".");
  return dot > 0 ? `${item.name.slice(0, dot)}-clean${item.name.slice(dot)}` : `${item.name}-clean`;
}

async function saveOne(item: Item) {
  if (item.outputId === undefined) return;
  try {
    const path = await save({ defaultPath: outName(item), filters: [{ name: "Image", extensions: [EXT[item.report!.after.format]] }] });
    if (!path) return;
    await writeFile(path, await api.output(item.outputId));
    item.saved = baseName(path);
    status(`Saved ${item.saved}.`);
  } catch (e) {
    status(`Could not save: ${e}`, true);
  }
  render();
}

async function saveAll() {
  const ready = items.filter((i) => i.outputId !== undefined);
  if (isAndroid) {
    for (const item of ready) await saveOne(item);
    return;
  }
  try {
    const dir = await open({ directory: true, recursive: false, title: "Choose a folder for the cleaned images" });
    if (!dir || Array.isArray(dir)) return;
    const sep = dir.includes("\\") ? "\\" : "/";
    let count = 0;
    for (const item of ready) {
      // Never overwrite: pick the first free name.
      let name = outName(item);
      const dot = name.lastIndexOf(".");
      for (let n = 2; await exists(dir + sep + name); n++) name = `${name.slice(0, dot).replace(/-\d+$/, "")}-${n}${name.slice(dot)}`;
      await writeFile(dir + sep + name, await api.output(item.outputId!));
      item.saved = name;
      count++;
    }
    status(`Saved ${count} image${count === 1 ? "" : "s"} to ${dir}.`);
  } catch (e) {
    status(`Could not save: ${e}`, true);
  }
  render();
}

// ---------- Rendering ----------
function status(text: string, error = false) {
  const el = $("status");
  el.textContent = text;
  el.classList.toggle("error", error);
}

function credentialsCard(c: Credentials): string {
  if (!c.present) return `<div class="card muted"><h4>Content Credentials</h4><p>None embedded in this file.</p></div>`;
  const badge: Record<string, [string, string, string]> = {
    trusted: ["ok", "Trusted signature", "The signature is intact and the signer is on a trust list."],
    valid: ["ok", "Signature intact", "The credentials match this file. The signer is not checked against a trust list, so their identity is not confirmed."],
    invalid: ["bad", "Validation failed", "The credentials do not match this file or the signature is broken. Do not rely on them."],
    unreadable: ["bad", "Could not be read", "The credentials are present but damaged or unsupported."],
  };
  const [tone, title, text] = badge[c.state] ?? badge.unreadable;
  const rows: [string, string | null][] = [
    ["Signed by", c.issuer], ["Signed at", c.signed_at], ["Made with", c.claim_generator],
    ["Title", c.title], ["Actions", c.actions.join(", ") || null], ["Algorithm", c.algorithm],
    ["Manifests", String(c.manifests)],
  ];
  return `<div class="card">
    <h4>Content Credentials <span class="badge ${tone}">${title}</span></h4>
    <p class="small">${text}</p>
    ${c.ai_generated ? `<p class="flag">These credentials declare AI-generated content.</p>` : ""}
    <dl>${rows.filter((r) => r[1]).map(([k, v]) => `<dt>${k}</dt><dd>${esc(v!)}</dd>`).join("")}</dl>
    ${c.problems.length ? `<details><summary>${c.problems.length} validation message${c.problems.length > 1 ? "s" : ""}</summary><ul class="small">${c.problems.map((p) => `<li>${esc(p)}</li>`).join("")}</ul></details>` : ""}
  </div>`;
}

function credAfter(r: CleanReport): string {
  const b = r.before.credentials, a = r.after.credentials;
  if (!b.present) return "";
  if (!a.present) return `<li class="yes">Content Credentials removed from this file</li>`;
  if (a.state === "valid" || a.state === "trusted") return `<li class="yes">Content Credentials kept and still pass validation</li>`;
  if (b.state === "valid" || b.state === "trusted") return `<li class="no">Content Credentials kept but now fail validation</li>`;
  return `<li class="no">Content Credentials kept; they did not pass validation before cleaning either</li>`;
}

function labels(blocks: Block[]): string {
  const counts = new Map<string, number>();
  for (const b of blocks) counts.set(b.label, (counts.get(b.label) ?? 0) + 1);
  return [...counts].map(([l, n]) => esc(l) + (n > 1 ? ` ×${n}` : "")).join(", ");
}

function reportCard(r: CleanReport): string {
  const kinds = [...new Set(r.before.blocks.map((b) => b.kind))];
  const removed = kinds.filter((k) => !r.after.blocks.some((b) => b.kind === k));
  const kept = [...new Set(r.after.blocks.map((b) => b.kind))];
  const credBroken = r.after.credentials.present && !["valid", "trusted"].includes(r.after.credentials.state) &&
    ["valid", "trusted"].includes(r.before.credentials.state);
  const ok = r.image_unchanged && r.still_present.length === 0 && !credBroken;
  return `<div class="card report ${ok ? "good" : "attention"}">
    <h4>Verified after cleaning</h4>
    <ul class="checks">
      <li class="${r.image_unchanged ? "yes" : "no"}">${r.image_unchanged ? "Image data identical to the original" : "Image data differs from the original"}</li>
      <li class="${r.still_present.length ? "no" : "yes"}">${r.still_present.length ? `Still present: ${r.still_present.map((k) => KIND_LABEL[k]).join(", ")}` : "Everything selected was removed"}</li>
      ${credAfter(r)}
      <li class="yes">${r.after.width} × ${r.after.height}${r.after.has_alpha ? ", transparency kept" : ""}${r.after.animated ? ", animation kept" : ""}</li>
    </ul>
    <p class="small">Removed: ${removed.map((k) => `${KIND_LABEL[k]} (${size(sumKind(r.before, k))})`).join(", ") || "nothing selected was present"}.
    Kept: ${kept.map((k) => KIND_LABEL[k]).join(", ") || "no metadata"}. ${r.bytes_removed > 0 ? `${size(r.bytes_removed)} smaller.` : ""}</p>
    ${r.after.exif && r.after.exif.fields.length ? `<p class="small">EXIF kept: ${r.after.exif.fields.map(([k, v]) => `${esc(k)} ${esc(v)}`).join(", ")}</p>` : ""}
    ${r.notes.map((n) => `<p class="note">${esc(n)}</p>`).join("")}
  </div>`;
}

function renderDetail() {
  const el = $("detail");
  const item = items[selected];
  if (!item) { el.innerHTML = ""; return; }
  if (!item.info) {
    el.innerHTML = `<div class="card bad-card"><h4>${esc(item.name)}</h4><p>${esc(item.error ?? "Unknown error")}</p></div>`;
    return;
  }
  const i = item.info;
  const groups = [...new Set(i.blocks.map((b) => b.kind))];
  const after = item.report?.after;
  el.innerHTML = `
    <div class="preview ${item.outputUrl ? "split" : ""}">
      <figure><img src="${item.url}" alt="Original" /><figcaption>Original · ${size(i.size)}</figcaption></figure>
      ${item.outputUrl && after ? `<figure class="appear"><img src="${item.outputUrl}" alt="Cleaned" /><figcaption>Cleaned · ${size(after.size)}</figcaption></figure>` : ""}
    </div>
    <div class="meta-line">${i.format.toUpperCase()} · ${i.width} × ${i.height}${i.has_alpha ? " · transparency" : ""}${i.animated ? " · animated" : ""}</div>
    ${item.report ? reportCard(item.report) : ""}
    <div class="card">
      <h4>Found in this file</h4>
      ${groups.length ? `<ul class="blocks">${groups.map((k) => `<li><span class="kind k-${k}">${KIND_LABEL[k]}</span><span>${labels(i.blocks.filter((b) => b.kind === k))}</span><span class="sz">${size(sumKind(i, k))}</span></li>`).join("")}</ul>` : "<p>No metadata found.</p>"}
    </div>
    ${i.exif && i.exif.fields.length ? `<div class="card"><h4>EXIF details <span class="small">${i.exif.tag_count} tags${i.exif.has_thumbnail ? ", embedded thumbnail" : ""}</span></h4><dl>${i.exif.fields.map(([k, v]) => `<dt>${esc(k)}</dt><dd class="${SENSITIVE.has(k) ? "sensitive" : ""}">${esc(v)}</dd>`).join("")}</dl></div>` : ""}
    ${credentialsCard(i.credentials)}
    ${item.outputId !== undefined ? `<button class="btn primary" id="save-one">${item.saved ? `Saved as ${esc(item.saved)} · Save again` : "Save cleaned image"}</button>` : ""}
  `;
  el.querySelector("#save-one")?.addEventListener("click", () => saveOne(item));
}

function renderList() {
  const ul = $("file-list");
  ul.innerHTML = items.map((it, idx) => {
    const r = it.report;
    const tag = it.error && !it.info ? `<span class="chip bad">Unsupported</span>`
      : r ? (r.image_unchanged && !r.still_present.length ? `<span class="chip ok">${it.saved ? "Saved" : "Clean"}</span>` : `<span class="chip bad">Check</span>`)
      : `<span class="chip">${it.info!.blocks.length} found</span>`;
    return `<li data-i="${idx}" class="${idx === selected ? "sel" : ""}">
      ${it.url ? `<img src="${it.outputUrl ?? it.url}" alt="" />` : `<div class="noimg"></div>`}
      <div><b>${esc(it.name)}</b><small>${size(it.bytes.length)}</small></div>${tag}</li>`;
  }).join("");
  ul.querySelectorAll("li").forEach((li) => li.addEventListener("click", () => { selected = Number(li.dataset.i); render(); }));
}

function renderOptions() {
  const box = $("toggles");
  if (!box.childElementCount) {
    box.innerHTML = TOGGLES.map((t) => `<label class="toggle ${t.key === "keep_orientation" ? "sep" : ""}">
      <input type="checkbox" data-k="${t.key}" ${options[t.key] ? "checked" : ""} /><span class="sw"></span>
      <span><b>${t.title}</b><small>${t.hint}</small></span></label>`).join("");
    box.querySelectorAll("input").forEach((inp) => inp.addEventListener("change", () => {
      options[inp.dataset.k as keyof Options] = inp.checked;
      render();
    }));
  }
  const exifOff = box.querySelector<HTMLInputElement>('input[data-k="keep_orientation"]')!;
  exifOff.disabled = !options.exif;
  const removes: Record<Kind, boolean> = {
    exif: options.exif, xmp: options.xmp, iptc: options.iptc, icc: options.icc, c2pa: options.c2pa,
    comment: options.comments, other: options.other,
  };
  const breaks = items.some((i) => i.info?.credentials.present && i.info.blocks.some((b) => b.kind !== "c2pa" && removes[b.kind]));
  const warn = $("c2pa-warning");
  warn.hidden = !(breaks && !options.c2pa);
  warn.textContent = "Content Credentials cover the whole file. Removing any other metadata while keeping them makes them fail validation. To keep them valid, turn off every other option.";
  const run = $<HTMLButtonElement>("run");
  const n = items.filter((i) => i.info).length;
  run.disabled = busy || n === 0;
  run.textContent = busy ? "Working…" : `Clean ${n} image${n === 1 ? "" : "s"}`;
  $("save-all").hidden = !items.some((i) => i.outputId !== undefined) || busy;
}

function render() {
  const has = items.length > 0;
  $("drop").classList.toggle("compact", has);
  $("workspace").hidden = !has;
  $("clear-list").hidden = !has;
  renderList();
  renderDetail();
  renderOptions();
}

// ---------- Self-test (native acceptance; inert unless CI places a sample) ----------
async function selftest() {
  const sample = toBytes(await invoke<ArrayBuffer | number[]>("selftest_sample"));
  if (sample.length === 0) return;
  const fail = (d: string) => invoke("selftest_done", { passed: false, detail: d });
  try {
    const bytes = sample;
    const before = await api.inspect(bytes);
    if (!before.credentials.present || before.credentials.state !== "valid") return fail(`credentials: ${JSON.stringify(before.credentials)}`);
    const all = { exif: true, xmp: true, iptc: true, icc: true, c2pa: true, comments: true, other: true, keep_orientation: false };
    const r = await api.clean(bytes, all);
    const out = await api.output(r.id);
    const after = await api.inspect(out);
    const ok = r.report.image_unchanged && after.blocks.length === 0 && !after.credentials.present &&
      after.image_fingerprint === before.image_fingerprint && out.length < bytes.length;
    await addFiles([{ name: "selftest.jpg", bytes }]);
    await invoke("selftest_done", { passed: ok, detail: `before ${before.blocks.length} blocks, after ${after.blocks.length}; ${bytes.length} -> ${out.length} bytes` });
  } catch (e) {
    await fail(String(e));
  }
}

// ---------- Wiring ----------
$("add").addEventListener("click", pickFiles);
$("pick").addEventListener("click", pickFiles);
$("drop").addEventListener("click", (e) => { if ((e.target as HTMLElement).id !== "pick" && !items.length) pickFiles(); });
$("run").addEventListener("click", runClean);
$("save-all").addEventListener("click", saveAll);
$("clear-list").addEventListener("click", async () => {
  const ids = items.map((i) => i.outputId).filter((x): x is number => x !== undefined);
  if (ids.length) await api.discard(ids);
  for (const i of items) { if (i.url) URL.revokeObjectURL(i.url); if (i.outputUrl) URL.revokeObjectURL(i.outputUrl); }
  items.length = 0;
  selected = -1;
  status("");
  render();
});
window.addEventListener("dragover", (e) => { e.preventDefault(); document.body.classList.add("dragging"); });
window.addEventListener("dragleave", (e) => { if (!e.relatedTarget) document.body.classList.remove("dragging"); });
window.addEventListener("drop", onDrop);
render();
selftest();
