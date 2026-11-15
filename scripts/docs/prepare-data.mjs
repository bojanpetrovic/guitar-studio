// A scratch data folder for the manual's screenshots: just a plugin list, cut down to
// free or built-in plugins (so the screenshots don't show anyone's paid collection).
//
//   node scripts/docs/prepare-data.mjs <the app's plugins.json> <scratch dir>
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [source, dir] = process.argv.slice(2);
if (!source || !dir) throw new Error("usage: prepare-data.mjs <plugins.json> <dir>");
const VENDORS = new Set(["Apple", "Steven Atkinson", "GuitarML", "MeldaProduction"]);
const SKIP = /^(AUAudioFilePlayer|AUNet|AURoundTrip|AUScheduled|AUSpeech|AURogerBeep|MNotepad|MCCGenerator|MRecorder)/;
const catalog = JSON.parse(readFileSync(source, "utf8"));
catalog.plugins = catalog.plugins.filter(
  (p) => VENDORS.has(p.vendor) && !SKIP.test(p.name) && ["effect", "tool"].includes(p.category),
);
catalog.errors = [];
rmSync(dir, { recursive: true, force: true });
mkdirSync(join(dir, "presets"), { recursive: true });
writeFileSync(join(dir, "plugins.json"), JSON.stringify(catalog, null, 1));
console.log(`${catalog.plugins.length} plugins in ${dir}`);
