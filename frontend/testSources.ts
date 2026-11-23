import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Read the stylesheet entry and its ordered, project-owned feature modules.
export function readProjectStyles(directory: string): string {
  return readFileSync(resolve(directory, "styles.css"), "utf8").replace(
    /@import "(\.\/styles\/[^\"]+)";/g,
    (_: string, path: string) => readFileSync(resolve(directory, path), "utf8"),
  );
}
