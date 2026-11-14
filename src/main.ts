import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { readFile, writeFile } from "@tauri-apps/plugin-fs";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { LANGS, lang, setLang, t, type Lang } from "./i18n";
import "./style.css";

type Kind = "exif" | "xmp" | "iptc" | "icc" | "c2pa" | "comment" | "embedded" | "other";
type Format = "jpeg" | "png" | "webp" | "heif" | "avif" | "wav" | "mp3" | "mp4" | "mov" | "m4a" | "pdf";

interface Block { kind: Kind; label: string; size: number }
interface Field { group: string; name: string; value: string; tag?: number }
interface Message { code: string; text: string }
interface Credentials {
  present: boolean; state: string; claim_generator: string | null; title: string | null;
  issuer: string | null; signed_at: string | null; algorithm: string | null;
  ai_generated: boolean; actions: string[]; manifests: number; problems: string[];
  source: string; profiles: Profile[]; store: ManifestInfo[];
}
interface Profile { id: string; state: string; failures: string[] }
interface IngredientInfo { title: string | null; format: string | null; relationship: string; manifest: string | null; failure: string[] }
interface ManifestInfo {
  label: string; active: boolean; claim_version: number | null; title: string | null; format: string | null;
  generator: string | null; issuer: string | null; signed_at: string | null; algorithm: string | null;
  assertions: string[]; ingredients: IngredientInfo[]; success: string[]; informational: string[]; failure: string[];
}
interface Inspection {
  format: Format; width: number; height: number; has_alpha: boolean; animated: boolean; size: number;
  blocks: Block[]; credentials: Credentials; image_fingerprint: string;
  embedded_fingerprint: string | null; warnings: string[]; details: Field[];
}
interface CleanReport {
  before: Inspection; after: Inspection; image_unchanged: boolean; still_present: Kind[];
  bytes_removed: number; verified: boolean; problems: Message[]; notes: Message[];
}
interface Options {
  exif: boolean; xmp: boolean; iptc: boolean; icc: boolean; c2pa: boolean; comments: boolean;
  other: boolean; embedded: boolean; keep_orientation: boolean; keep_dates: boolean; keep_resolution: boolean;
}
interface AppError { code: string; text: string }
/** Where an image comes from. Bytes are read only when needed and never kept. */
type Source = { path: string } | { uri: string } | { file: File };
interface Item {
  id: number; name: string; source: Source; thumb: string;
  info?: Inspection; error?: AppError; report?: CleanReport; outputId?: number; saved?: string;
}

const VERSION = "0.4.0";
const MIME: Record<Format, string> = {
  jpeg: "image/jpeg", png: "image/png", webp: "image/webp", heif: "image/heic", avif: "image/avif",
  wav: "audio/wav", mp3: "audio/mpeg", mp4: "video/mp4", mov: "video/quicktime", m4a: "audio/mp4", pdf: "application/pdf",
};
const EXT: Record<Format, string> = {
  jpeg: "jpg", png: "png", webp: "webp", heif: "heic", avif: "avif",
  wav: "wav", mp3: "mp3", mp4: "mp4", mov: "mov", m4a: "m4a", pdf: "pdf",
};
const ALL_EXT = ["jpg", "jpeg", "png", "webp", "heic", "heif", "avif", "wav", "mp3", "mp4", "m4v", "mov", "m4a", "pdf"];
/** Formats the web view can draw as a thumbnail. */
const PREVIEW = new Set<Format>(["jpeg", "png", "webp", "avif"]);
/** Fields that commonly identify a person, device or place. */
const SENSITIVE = /GPS|Latitude|Longitude|Altitude|serial|owner|Artist|Creator|Copyright|Camera make|Camera model|Host computer|Unique image ID|By-line|City|Country|location/i;
const REMOVE_KEYS: (keyof Options)[] = ["exif", "xmp", "iptc", "comments", "other", "embedded", "icc", "c2pa"];
const KEEP_KEYS: (keyof Options)[] = ["keep_orientation", "keep_dates", "keep_resolution"];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const isAndroid = /Android/i.test(navigator.userAgent);
const items: Item[] = [];
let selected = -1;
let nextId = 1;
let busy = false;
let keepFileTime = false;
let detailFilter = "";
const options: Options = {
  exif: true, xmp: true, iptc: true, icc: false, c2pa: false, comments: true, other: true, embedded: false,
  keep_orientation: true, keep_dates: false, keep_resolution: true,
};

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}
function size(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}
const kindLabel = (k: Kind) => t(`k.${k}`);
const sumKind = (info: Inspection, kind: Kind) =>
  info.blocks.filter((b) => b.kind === kind).reduce((a, b) => a + b.size, 0);
const msgText = (m: Message) => (t(`m.${m.code}`) === `m.${m.code}` ? m.text : t(`m.${m.code}`));

/** Native errors arrive as "code|message". */
function toError(e: unknown): AppError {
  const s = String(e);
  const bar = s.indexOf("|");
  return bar > 0 && bar < 20 ? { code: s.slice(0, bar), text: s.slice(bar + 1) } : { code: "io", text: s };
}
const errText = (e: AppError) => (t(`err.${e.code}`) === `err.${e.code}` ? e.text : t(`err.${e.code}`));

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

// ---------- Reading sources ----------
async function bytesOf(src: Source): Promise<Uint8Array> {
  if ("path" in src) return toBytes(await invoke<ArrayBuffer | number[]>("read_path", { path: src.path }));
  if ("uri" in src) return readFile(src.uri);
  return new Uint8Array(await src.file.arrayBuffer());
}
async function inspectSource(src: Source): Promise<Inspection> {
  return "path" in src ? invoke<Inspection>("inspect_path", { path: src.path }) : api.inspect(await bytesOf(src));
}
async function cleanSource(src: Source, o: Options) {
  if ("path" in src) {
    await invoke("set_options", { options: o });
    return invoke<{ id: number; report: CleanReport }>("clean_path", { path: src.path });
  }
  return api.clean(await bytesOf(src), o);
}
/** A small preview, so full-size images are not kept in memory. */
async function thumbnail(src: Source, format: Format): Promise<string> {
  const blob = "file" in src ? src.file : new Blob([(await bytesOf(src)) as BlobPart], { type: MIME[format] });
  try {
    const bmp = await createImageBitmap(blob, { resizeWidth: 960, resizeQuality: "medium" });
    const canvas = document.createElement("canvas");
    canvas.width = bmp.width;
    canvas.height = bmp.height;
    canvas.getContext("2d")!.drawImage(bmp, 0, 0);
    bmp.close();
    const out = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/webp", 0.85));
    return out ? URL.createObjectURL(out) : "";
  } catch {
    return "";
  }
}

// ---------- Adding files ----------
function baseName(path: string): string {
  let name = path.split(/[\\/]/).pop() || "image";
  try { name = decodeURIComponent(name); } catch { /* keep raw */ }
  return name.split(/[\\/:]/).pop() || "image";
}

async function addSources(sources: { name: string; source: Source }[]) {
  let n = 0;
  for (const f of sources) {
    status(t("reading", { i: ++n, n: sources.length }));
    const item: Item = { id: nextId++, name: f.name, source: f.source, thumb: "" };
    try {
      item.info = await inspectSource(f.source);
      if (PREVIEW.has(item.info.format)) item.thumb = await thumbnail(f.source, item.info.format).catch(() => "");
      if (!/\.[a-z0-9]{3,4}$/i.test(item.name)) item.name += "." + EXT[item.info.format];
    } catch (e) {
      item.error = toError(e);
    }
    items.push(item);
    if (selected < 0) selected = 0;
    if (n % 10 === 0) render();
  }
  const failed = items.filter((i) => !i.info).length;
  status(
    !sources.length ? t("noneFound")
      : failed ? t("errorsN", { n: failed })
      : sources.length === 1 ? t("added1") : t("added", { n: sources.length }),
    failed > 0,
  );
  render();
}

/** Desktop: expand folders recursively, then add every supported image. */
async function addPaths(paths: string[]) {
  const files = await invoke<string[]>("expand_paths", { paths });
  await addSources(files.map((p) => ({ name: baseName(p), source: { path: p } })));
}

/** Files shared into the app from another app (Android share sheet). */
async function takeShared() {
  if (!isAndroid) return;
  try {
    const paths = await invoke<string[]>("take_shared");
    if (paths.length) await addSources(paths.map((p) => ({ name: baseName(p), source: { path: p } })));
  } catch {
    /* nothing shared */
  }
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") void takeShared();
});

async function pickFiles() {
  try {
    const picked = await open({ multiple: true, filters: [{ name: "Media", extensions: ALL_EXT }] });
    if (!picked) return;
    const paths = Array.isArray(picked) ? picked : [picked];
    if (isAndroid) await addSources(paths.map((p) => ({ name: baseName(p), source: { uri: p } })));
    else await addPaths(paths);
  } catch (e) {
    status(t("couldNotOpen", { e: errText(toError(e)) }), true);
  }
}

async function pickFolder() {
  try {
    const dir = await open({ directory: true });
    if (dir && !Array.isArray(dir)) await addPaths([dir]);
  } catch (e) {
    status(t("couldNotOpen", { e: errText(toError(e)) }), true);
  }
}

/** Fallback for webviews without native drag and drop. */
async function onDrop(ev: DragEvent) {
  ev.preventDefault();
  document.body.classList.remove("dragging");
  const list = Array.from(ev.dataTransfer?.files ?? []);
  if (list.length) await addSources(list.map((f) => ({ name: f.name, source: { file: f } })));
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
    status(t("cleaning", { i: ++done, n: todo.length }));
    item.report = undefined;
    item.outputId = undefined;
    item.saved = undefined;
    item.error = undefined;
    try {
      const r = await cleanSource(item.source, options);
      item.outputId = r.id;
      item.report = r.report;
    } catch (e) {
      item.error = toError(e);
    }
    if (done % 10 === 0) render();
  }
  busy = false;
  const failed = todo.filter((i) => !i.report?.verified).length;
  status(failed ? t("cleanedSome", { ok: todo.length - failed, bad: failed }) : t("cleanedAll", { n: todo.length }), failed > 0);
  render();
}

function outName(item: Item): string {
  const dot = item.name.lastIndexOf(".");
  return dot > 0 ? `${item.name.slice(0, dot)}-clean${item.name.slice(dot)}` : `${item.name}-clean`;
}

const savable = (i: Item) => i.outputId !== undefined && i.report?.verified === true;

async function saveOne(item: Item) {
  if (!savable(item)) return;
  try {
    if (isAndroid) {
      const path = await save({ defaultPath: outName(item), filters: [{ name: item.report!.after.format.toUpperCase(), extensions: [EXT[item.report!.after.format]] }] });
      if (!path) return;
      await writeFile(path, await api.output(item.outputId!));
      item.saved = baseName(path);
    } else {
      const name = await invoke<string | null>("save_one", { id: item.outputId, name: outName(item), keepTime: keepFileTime });
      if (!name) return;
      item.saved = name;
    }
    status(t("saved", { name: item.saved }));
  } catch (e) {
    status(t("couldNotSave", { e: errText(toError(e)) }), true);
  }
  render();
}

async function saveAll() {
  const ready = items.filter(savable);
  const skipped = items.filter((i) => i.outputId !== undefined).length - ready.length;
  if (isAndroid) {
    for (const item of ready) await saveOne(item);
    return;
  }
  try {
    const result = await invoke<[string, { id: number; name: string }[]] | null>("save_all", {
      items: ready.map((i) => [i.outputId, outName(i)]),
      keepTime: keepFileTime,
    });
    if (!result) return;
    const [dir, saved] = result;
    for (const s of saved) {
      const item = items.find((i) => i.outputId === s.id);
      if (item) item.saved = s.name;
    }
    status(t("savedTo", { n: saved.length, dir }) + (skipped ? " " + t("skipped", { n: skipped }) : ""));
  } catch (e) {
    status(t("couldNotSave", { e: errText(toError(e)) }), true);
  }
  render();
}

// ---------- JSON reports ----------
function reportFor(list: Item[]) {
  return {
    generator: "Nexonic Clear",
    version: VERSION,
    generated_at: new Date().toISOString(),
    options,
    note: "Lists the metadata found before cleaning. Treat this report as private if the images were.",
    files: list.map((i) => ({
      name: i.name,
      status: i.error ? "error" : i.report ? (i.report.verified ? "verified" : "not_verified") : "inspected",
      error: i.error ?? null,
      saved_as: i.saved ?? null,
      format: i.info?.format ?? null,
      width: i.info?.width ?? null,
      height: i.info?.height ?? null,
      image_sha256: i.info?.image_fingerprint ?? null,
      size_before: i.info?.size ?? null,
      size_after: i.report?.after.size ?? null,
      problems: i.report?.problems ?? [],
      notes: i.report?.notes ?? [],
      before: i.info ? { blocks: i.info.blocks, details: i.info.details, credentials: i.info.credentials } : null,
      after: i.report ? { blocks: i.report.after.blocks, details: i.report.after.details, credentials: i.report.after.credentials } : null,
    })),
  };
}

async function exportReport(list: Item[]) {
  if (!list.length) return;
  const json = JSON.stringify(reportFor(list), null, 2);
  const name = list.length === 1 ? `${list[0].name.replace(/\.[^.]+$/, "")}-report.json` : "nexonic-clear-report.json";
  try {
    let savedName: string | null = null;
    if (isAndroid) {
      const path = await save({ defaultPath: name, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (path) {
        await writeFile(path, new TextEncoder().encode(json));
        savedName = baseName(path);
      }
    } else {
      savedName = await invoke<string | null>("save_report", { json, name });
    }
    if (savedName) status(t("reportSaved", { name: savedName }));
  } catch (e) {
    status(t("couldNotSave", { e: errText(toError(e)) }), true);
  }
}

// ---------- Rendering ----------
function status(text: string, error = false) {
  const el = $("status");
  el.textContent = text;
  el.classList.toggle("error", error);
}

function credentialsCard(c: Credentials): string {
  if (!c.present) return `<div class="card muted"><h4>${t("credTitle")}</h4><p>${t("credNone")}</p></div>`;
  const state = ["trusted", "valid", "invalid", "unreadable"].includes(c.state) ? c.state : "unreadable";
  const tone = state === "trusted" || state === "valid" ? "ok" : "bad";
  const rows: [string, string | null][] = [
    [t("signedBy"), c.issuer], [t("signedAt"), c.signed_at], [t("madeWith"), c.claim_generator],
    [t("title"), c.title], [t("actions"), c.actions.join(", ") || null], [t("algorithm"), c.algorithm],
    [t("manifests"), String(c.manifests)],
  ];
  return `<div class="card">
    <h4>${t("credTitle")} <span class="badge ${tone}">${t(`cred.${state}`)}</span></h4>
    <p class="small">${t(`credx.${state}`)}</p>
    ${c.ai_generated ? `<p class="flag">${t("credAi")}</p>` : ""}
    <dl>${rows.filter((r) => r[1]).map(([k, v]) => `<dt>${k}</dt><dd>${esc(v!)}</dd>`).join("")}</dl>
    ${c.source === "sidecar" ? `<p class="small">${t("credSidecar")}</p>` : ""}
    ${profilesTable(c.profiles ?? [])}
    ${c.problems.length ? `<details><summary>${t("credMessages", { n: c.problems.length })}</summary><ul class="small">${c.problems.map((p) => `<li>${esc(p)}</li>`).join("")}</ul></details>` : ""}
    ${(c.store ?? []).length ? `<details><summary>${t("storeTitle", { n: c.store.length })}</summary>${c.store.map(manifestBlock).join("")}</details>` : ""}
  </div>`;
}

function stateBadge(state: string): string {
  const s = ["trusted", "valid", "invalid", "unreadable"].includes(state) ? state : "unreadable";
  return `<span class="badge ${s === "trusted" || s === "valid" ? "ok" : "bad"}">${t(`cred.${s}`)}</span>`;
}

function profilesTable(list: Profile[]): string {
  if (!list.length) return "";
  return `<table class="profiles"><thead><tr><th>${t("profile")}</th><th>${t("result")}</th></tr></thead><tbody>${list
    .map((p) => `<tr><td>${t(`profile.${p.id}`)}</td><td>${stateBadge(p.state)}${p.failures.length
      ? `<ul class="small">${p.failures.slice(0, 8).map((f) => `<li>${esc(f)}</li>`).join("")}</ul>` : ""}</td></tr>`)
    .join("")}</tbody></table>`;
}

function manifestBlock(m: ManifestInfo): string {
  const rows: [string, string | null][] = [
    [t("mLabel"), m.label], [t("claimVersion"), m.claim_version != null ? `v${m.claim_version}` : null],
    [t("title"), m.title], [t("format"), m.format], [t("madeWith"), m.generator], [t("signedBy"), m.issuer],
    [t("signedAt"), m.signed_at], [t("algorithm"), m.algorithm],
  ];
  const list = (head: string, items: string[]) => items.length
    ? `<p class="small"><strong>${head}</strong></p><ul class="small">${items.slice(0, 60).map((x) => `<li>${esc(x)}</li>`).join("")}</ul>` : "";
  return `<div class="manifest${m.active ? " active" : ""}">
    <h5>${m.active ? t("mActive") : t("mIngredient")}</h5>
    <dl>${rows.filter((r) => r[1]).map(([k, v]) => `<dt>${k}</dt><dd>${esc(v!)}</dd>`).join("")}</dl>
    ${list(t("assertions"), m.assertions)}
    ${list(t("ingredients"), m.ingredients.map((i) => `${i.title ?? "?"} (${i.relationship}${i.format ? `, ${i.format}` : ""})${i.failure.length ? ` – ${i.failure.join("; ")}` : ""}`))}
    ${list(t("codesFailure"), m.failure)}
    ${list(t("codesInfo"), m.informational)}
    ${list(t("codesSuccess"), m.success)}
  </div>`;
}

function credAfter(r: CleanReport): string {
  const b = r.before.credentials, a = r.after.credentials;
  const good = (s: string) => s === "valid" || s === "trusted";
  if (!b.present) return "";
  if (!a.present) return `<li class="yes">${t("credRemoved")}</li>`;
  if (good(a.state)) return `<li class="yes">${t("credValid")}</li>`;
  if (good(b.state)) return `<li class="no">${t("credBroken")}</li>`;
  return `<li class="no">${t("credWasInvalid")}</li>`;
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
  const extras = [r.after.has_alpha ? t("transparencyKept") : "", r.after.animated ? t("animationKept") : ""].filter(Boolean);
  return `<div class="card report ${r.verified && !credBroken ? "good" : "attention"}">
    <h4>${r.verified ? t("verifiedAfter") : t("verificationFailed")}</h4>
    ${r.problems.map((p) => `<p class="note bad-note">${esc(msgText(p))}</p>`).join("")}
    <ul class="checks">
      <li class="${r.image_unchanged ? "yes" : "no"}">${r.image_unchanged ? t("imageIdentical") : t("imageDiffers")}</li>
      <li class="${r.still_present.length ? "no" : "yes"}">${r.still_present.length ? t("stillPresent", { list: r.still_present.map(kindLabel).join(", ") }) : t("allRemoved")}</li>
      ${credAfter(r)}
      <li class="yes">${r.after.width ? `${r.after.width} × ${r.after.height}` : r.after.format.toUpperCase()}${extras.length ? ", " + extras.join(", ") : ""}</li>
    </ul>
    <p class="small">${t("removedList", { list: removed.map((k) => `${kindLabel(k)} (${size(sumKind(r.before, k))})`).join(", ") || t("nothingRemoved") })}
    ${t("keptList", { list: kept.map(kindLabel).join(", ") || t("noMeta") })} ${r.bytes_removed > 0 ? t("smaller", { size: size(r.bytes_removed) }) : ""}</p>
    ${r.notes.map((n) => `<p class="note">${esc(msgText(n))}</p>`).join("")}
  </div>`;
}

const fieldKey = (f: Field) => `${f.group}\u0000${f.name}`;

/** Field-by-field comparison: what was there before and what is left after. */
function compareCard(r: CleanReport): string {
  const after = new Map(r.after.details.map((f) => [fieldKey(f), f.value]));
  const rows = r.before.details;
  if (!rows.length) return "";
  const keptN = rows.filter((f) => after.has(fieldKey(f))).length;
  const shown = rows.slice(0, 300);
  return `<details class="card compare">
    <summary><h4>${t("compare")} <span class="small">${t("compareSummary", { removed: rows.length - keptN, kept: keptN })}</span></h4></summary>
    <table><thead><tr><th>${t("field")}</th><th>${t("before")}</th><th>${t("after")}</th></tr></thead><tbody>
    ${shown.map((f) => {
      const a = after.get(fieldKey(f));
      return `<tr class="${a === undefined ? "gone" : "kept"}"><td><span class="grp">${esc(f.group)}</span> ${esc(f.name)}</td><td>${esc(f.value)}</td><td>${a === undefined ? `<em>${t("removedTag")}</em>` : esc(a)}</td></tr>`;
    }).join("")}
    </tbody></table></details>`;
}

/** Every readable field, grouped, with a filter. */
function detailsCard(i: Inspection): string {
  if (!i.details.length) return `<div class="card muted"><h4>${t("details")}</h4><p>${t("noFields")}</p></div>`;
  const q = detailFilter.toLowerCase();
  const list = q ? i.details.filter((f) => `${f.group} ${f.name} ${f.value}`.toLowerCase().includes(q)) : i.details;
  const groups = [...new Set(list.map((f) => f.group))];
  return `<div class="card details-card">
    <h4>${t("details")} <span class="small">${t("fieldsN", { n: i.details.length })}</span></h4>
    <input id="detail-filter" class="filter" type="search" placeholder="${t("filter")}" value="${esc(detailFilter)}" />
    ${groups.map((g) => {
      const fs = list.filter((f) => f.group === g);
      return `<details ${q || groups.length <= 2 ? "open" : ""}><summary>${esc(g)} <span class="small">${fs.length}</span></summary>
        <dl>${fs.map((f) => `<dt>${esc(f.name)}</dt><dd class="${SENSITIVE.test(f.name) || f.group === "GPS" ? "sensitive" : ""}">${esc(f.value)}</dd>`).join("")}</dl></details>`;
    }).join("")}
  </div>`;
}

function errorCard(item: Item): string {
  const e = item.error!;
  return `<div class="card bad-card"><h4>${esc(item.name)}</h4><p>${esc(errText(e))}</p>
    <details><summary>${t("errDetail")}</summary><p class="small mono">${esc(e.code)}: ${esc(e.text)}</p></details></div>`;
}

function renderDetail() {
  const el = $("detail");
  const item = items[selected];
  if (!item) { el.innerHTML = ""; return; }
  if (!item.info) { el.innerHTML = errorCard(item); return; }
  const i = item.info;
  const groups = [...new Set(i.blocks.map((b) => b.kind))];
  const r = item.report;
  const showAfter = r?.verified;
  const preview = item.thumb ? `<img src="${item.thumb}" alt="" />` : `<div class="noprev">${t("noPreview")}</div>`;
  el.innerHTML = `
    <div class="preview ${showAfter ? "split" : ""}">
      <figure>${preview}<figcaption>${t("original")} · ${size(i.size)}</figcaption></figure>
      ${showAfter ? `<figure class="appear">${preview}<figcaption>${t("cleaned")} · ${size(r!.after.size)} · ${t("identicalPixels")}</figcaption></figure>` : ""}
    </div>
    ${i.warnings.map((w) => `<p class="note">${esc(w)}</p>`).join("")}
    <div class="meta-line">${i.format.toUpperCase()}${i.width ? ` · ${i.width} × ${i.height}` : ""}${i.has_alpha ? " · alpha" : ""}${i.animated ? " · ▶" : ""}</div>
    ${item.error ? errorCard(item) : ""}
    ${r ? reportCard(r) : ""}
    <div class="actions">
      ${savable(item) ? `<button class="btn primary" id="save-one">${item.saved ? t("saveAgain", { name: esc(item.saved) }) : t("saveOne")}</button>` : ""}
      <button class="btn" id="export-one">${t("exportReport")}</button>
    </div>
    ${r && !r.verified ? `<p class="note bad-note">${t("notSaved")}</p>` : ""}
    ${r ? compareCard(r) : ""}
    <div class="card">
      <h4>${t("found")}</h4>
      ${groups.length ? `<ul class="blocks">${groups.map((k) => `<li><span class="kind k-${k}">${kindLabel(k)}</span><span>${labels(i.blocks.filter((b) => b.kind === k))}</span><span class="sz">${size(sumKind(i, k))}</span></li>`).join("")}</ul>` : `<p>${t("noMetadata")}</p>`}
    </div>
    ${detailsCard(i)}
    ${credentialsCard(i.credentials)}
  `;
  el.querySelector("#save-one")?.addEventListener("click", () => saveOne(item));
  el.querySelector("#export-one")?.addEventListener("click", () => exportReport([item]));
  const filter = el.querySelector<HTMLInputElement>("#detail-filter");
  filter?.addEventListener("input", () => {
    detailFilter = filter.value;
    const pos = filter.selectionStart;
    renderDetail();
    const again = $("detail").querySelector<HTMLInputElement>("#detail-filter");
    again?.focus();
    if (pos !== null) again?.setSelectionRange(pos, pos);
  });
}

function renderList() {
  const ul = $("file-list");
  ul.innerHTML = items.map((it, idx) => {
    const r = it.report;
    const tag = it.error && !it.info ? `<span class="chip bad">${it.error.code === "unsupported" ? t("chipUnsupported") : t("chipFailed")}</span>`
      : it.error && !r ? `<span class="chip bad">${t("chipFailed")}</span>`
      : r ? (r.verified ? `<span class="chip ok">${it.saved ? t("chipSaved") : t("chipVerified")}</span>` : `<span class="chip bad">${t("chipNotVerified")}</span>`)
      : `<span class="chip">${t("chipFound", { n: it.info!.blocks.length })}</span>`;
    return `<li data-i="${idx}" class="${idx === selected ? "sel" : ""}">
      ${it.thumb ? `<img src="${it.thumb}" alt="" />` : `<div class="noimg"></div>`}
      <div><b>${esc(it.name)}</b><small>${it.info ? size(it.info.size) : ""}</small></div>${tag}</li>`;
  }).join("");
  ul.querySelectorAll("li").forEach((li) => li.addEventListener("click", () => { selected = Number(li.dataset.i); render(); }));
}

function toggleHtml(key: string, checked: boolean, sep = false) {
  return `<label class="toggle ${sep ? "sep" : ""}">
    <input type="checkbox" data-k="${key}" ${checked ? "checked" : ""} /><span class="sw"></span>
    <span><b>${t(`t.${key}`)}</b><small>${t(`h.${key}`)}</small></span></label>`;
}

function renderOptions() {
  const box = $("toggles");
  if (box.dataset.lang !== lang()) {
    box.dataset.lang = lang();
    box.innerHTML = REMOVE_KEYS.map((k) => toggleHtml(k, options[k])).join("") +
      `<h3 class="keep-h">${t("keep")}</h3>` +
      KEEP_KEYS.map((k) => toggleHtml(k, options[k])).join("") +
      (isAndroid ? "" : toggleHtml("keep_time", keepFileTime));
    box.querySelectorAll("input").forEach((inp) => inp.addEventListener("change", () => {
      if (inp.dataset.k === "keep_time") keepFileTime = inp.checked;
      else options[inp.dataset.k as keyof Options] = inp.checked;
      render();
    }));
  }
  for (const k of KEEP_KEYS) box.querySelector<HTMLInputElement>(`input[data-k="${k}"]`)!.disabled = !options.exif;
  const removes: Record<Kind, boolean> = {
    exif: options.exif, xmp: options.xmp, iptc: options.iptc, icc: options.icc, c2pa: options.c2pa,
    comment: options.comments, embedded: options.embedded, other: options.other,
  };
  const breaks = items.some((i) => i.info?.credentials.present && i.info.blocks.some((b) => b.kind !== "c2pa" && removes[b.kind]));
  const warn = $("c2pa-warning");
  warn.hidden = !(breaks && !options.c2pa);
  warn.textContent = t("c2paWarning");
  const run = $<HTMLButtonElement>("run");
  const n = items.filter((i) => i.info).length;
  run.disabled = busy || n === 0;
  run.textContent = busy ? t("working") : n === 1 ? t("clean1") : t("cleanN", { n });
  $("save-all").hidden = !items.some(savable) || busy;
  $("export-all").hidden = items.length < 2 || busy;
}

function applyStatic() {
  document.querySelectorAll<HTMLElement>("[data-t]").forEach((el) => { el.textContent = t(el.dataset.t!); });
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
    if (!before.credentials.present || before.credentials.state !== "valid" || before.credentials.profiles.length !== 3 || !before.credentials.store.length) return fail(`credentials: ${JSON.stringify(before.credentials)}`);
    const all: Options = { exif: true, xmp: true, iptc: true, icc: true, c2pa: true, comments: true, other: true, embedded: true, keep_orientation: false, keep_dates: false, keep_resolution: false };
    const r = await api.clean(bytes, all);
    const out = await api.output(r.id);
    const after = await api.inspect(out);
    // Desktop also reads and cleans by path, the way dropped files are handled.
    let pathOk = true;
    const samplePath = isAndroid ? null : await invoke<string | null>("selftest_path");
    if (samplePath) {
      const viaPath = await cleanSource({ path: samplePath }, all);
      const inspected = await inspectSource({ path: samplePath });
      pathOk = viaPath.report.verified && inspected.image_fingerprint === before.image_fingerprint &&
        viaPath.report.after.blocks.length === 0;
    }
    const ok = pathOk && r.report.verified && r.report.image_unchanged && after.blocks.length === 0 && !after.credentials.present &&
      after.details.length === 0 && after.image_fingerprint === before.image_fingerprint && out.length < bytes.length;
    await addSources([{ name: "selftest.jpg", source: { file: new File([bytes as BlobPart], "selftest.jpg") } }]);
    await invoke("selftest_done", { passed: ok, detail: `before ${before.blocks.length} blocks, after ${after.blocks.length}; ${bytes.length} -> ${out.length} bytes; path flow ${samplePath ? (pathOk ? "ok" : "failed") : "n/a"}` });
  } catch (e) {
    await fail(String(e));
  }
}

// ---------- Wiring ----------
const langSel = $<HTMLSelectElement>("lang");
langSel.innerHTML = LANGS.map(([code, name]) => `<option value="${code}" ${code === lang() ? "selected" : ""}>${name}</option>`).join("");
langSel.addEventListener("change", () => { setLang(langSel.value as Lang); applyStatic(); render(); });
$("add").addEventListener("click", pickFiles);
const folderBtn = $("add-folder");
folderBtn.hidden = isAndroid;
folderBtn.addEventListener("click", pickFolder);
$("pick").addEventListener("click", pickFiles);
$("drop").addEventListener("click", (e) => { if ((e.target as HTMLElement).id !== "pick" && !items.length) pickFiles(); });
$("run").addEventListener("click", runClean);
$("save-all").addEventListener("click", saveAll);
$("export-all").addEventListener("click", () => exportReport(items));
$("clear-list").addEventListener("click", async () => {
  const ids = items.map((i) => i.outputId).filter((x): x is number => x !== undefined);
  if (ids.length) await api.discard(ids);
  for (const i of items) if (i.thumb) URL.revokeObjectURL(i.thumb);
  items.length = 0;
  selected = -1;
  status("");
  render();
});
// Desktop: native drag and drop delivers real paths, including folders.
if (!isAndroid) {
  getCurrentWebview()
    .onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") document.body.classList.add("dragging");
      else if (p.type === "leave") document.body.classList.remove("dragging");
      else if (p.type === "drop") {
        document.body.classList.remove("dragging");
        if (p.paths.length) void addPaths(p.paths);
      }
    })
    .catch(() => { /* not running inside the app */ });
}
window.addEventListener("dragover", (e) => { e.preventDefault(); document.body.classList.add("dragging"); });
window.addEventListener("dragleave", (e) => { if (!e.relatedTarget) document.body.classList.remove("dragging"); });
window.addEventListener("drop", onDrop);
applyStatic();
render();
selftest();

void takeShared();
