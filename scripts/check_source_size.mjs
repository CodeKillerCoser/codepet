import { execFileSync } from "node:child_process";
import { readFileSync, existsSync } from "node:fs";
import { extname } from "node:path";

const review = process.argv.includes("--review");
const sourceExtensions = new Set([".svelte", ".ts", ".rs", ".css", ".mjs", ".py", ".m"]);
const gitLines = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim().split("\n").filter(Boolean);
const files = [...new Set(gitLines("ls-files", "--cached", "--others", "--exclude-standard"))];
const changed = new Set(gitLines("diff", "--name-only", "HEAD"));
const base = process.env.CODEPET_SIZE_BASE;
if (base && !/^0+$/.test(base)) {
  for (const file of gitLines("diff", "--name-only", base, "HEAD")) changed.add(file);
}
for (const file of gitLines("ls-files", "--others", "--exclude-standard")) changed.add(file);
const generated = file => /(^|\/)(vendor|target|node_modules|artifacts)\//.test(file)
  || /(^|\/)generated\.[^/]+$/.test(file)
  || /^(sdk\/typescript\/.*\/src\/generated|examples\/providers\/independent\/packages\/[^/]+\/provider\.mjs$)/.test(file);
let failures = 0, warnings = 0;
for (const file of files) {
  if (!sourceExtensions.has(extname(file)) || generated(file) || !existsSync(file)) continue;
  // Enforce all frontend files and any other source touched by this change.
  // --review also reports existing backend debt without silently exempting it from review.
  const enforced = file.startsWith("frontend/") || changed.has(file);
  if (!review && !enforced) continue;
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n").length - Number(text.endsWith("\n"));
  if (lines <= 1500) continue;
  if (lines > 2000 && enforced) {
    console.error(`ERROR ${file}: ${lines} lines; split by responsibility (limit 2000).`);
    failures += 1;
  } else {
    console.log(`REVIEW ${file}: ${lines} lines${enforced ? "; plan a split before further growth" : "; existing source outside the UI refactor"}.`);
    warnings += 1;
  }
}
console.log(`Source-size check (frontend + changed source): ${failures} violations, ${warnings} review items.`);
process.exitCode = failures ? 1 : 0;
