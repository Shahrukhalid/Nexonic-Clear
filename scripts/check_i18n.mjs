// Fails if any language is missing a key or changes a {placeholder}.
import fs from "node:fs";
const src = fs.readFileSync(new URL("../src/i18n.ts", import.meta.url), "utf8");
const dicts = {};
for (const m of src.matchAll(/const (\w+): Dict = (\{[\s\S]*?\n\});/g)) {
  dicts[m[1]] = Function(`return (${m[2]});`)();
}
const en = dicts.en;
const holes = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((x) => x[1]).sort().join(",");
let bad = 0;
for (const [lang, d] of Object.entries(dicts)) {
  for (const k of Object.keys(en)) {
    if (!(k in d)) { console.error(`${lang}: missing ${k}`); bad++; }
    else if (holes(d[k]) !== holes(en[k])) { console.error(`${lang}: placeholders differ in ${k}`); bad++; }
  }
  for (const k of Object.keys(d)) if (!(k in en)) { console.error(`${lang}: unknown key ${k}`); bad++; }
}
console.log(`${Object.keys(dicts).length} languages, ${Object.keys(en).length} keys, ${bad} problems`);
process.exit(bad ? 1 : 0);
