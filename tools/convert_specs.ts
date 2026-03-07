#!/usr/bin/env -S deno run --allow-read --allow-write

/**
 * Convert Fig/Withfig autocomplete TypeScript specs to JSON.
 *
 * Usage:
 *   deno run --allow-read --allow-write tools/convert_specs.ts <fig-autocomplete-src-dir> <output-dir>
 *
 * Example:
 *   git clone https://github.com/withfig/autocomplete /tmp/autocomplete
 *   deno run --allow-read --allow-write tools/convert_specs.ts /tmp/autocomplete/src specs/
 *
 * The Fig specs export a `default` which is the spec object. This script
 * uses a regex-based extraction approach to avoid needing the full Fig
 * type system — it extracts the object literal from each TS file.
 */

const PRIORITY_COMMANDS = [
  "git", "docker", "cargo", "npm", "yarn", "pnpm", "node", "python", "python3",
  "pip", "pip3", "brew", "apt", "apt-get", "dnf", "yum", "pacman",
  "ls", "cd", "cp", "mv", "rm", "mkdir", "cat", "less", "grep", "find",
  "curl", "wget", "ssh", "scp", "rsync",
  "make", "cmake", "go", "rustup", "rustc",
  "kubectl", "helm", "terraform", "aws", "gcloud", "az",
  "vim", "nvim", "nano", "code",
  "tar", "zip", "unzip", "gzip",
  "systemctl", "journalctl",
  "tmux", "screen",
  "chmod", "chown", "kill", "ps", "top", "htop",
  "sed", "awk", "sort", "uniq", "wc", "head", "tail",
  "man", "which", "whereis", "env", "export",
  "gh", "jq", "deno", "bun",
];

async function convertSpec(inputPath: string, outputDir: string): Promise<boolean> {
  try {
    const content = await Deno.readTextFile(inputPath);

    // Try to extract the spec object from the TS file.
    // Fig specs typically have: const completionSpec: Fig.Spec = { ... };
    // or: export default { ... } satisfies Fig.Spec;

    // Strategy: use dynamic import with esbuild-like transform
    // Simpler: write a minimal extraction

    // Find the main object literal
    let specObj: unknown;

    // Try: evaluate as JS (strip type annotations naively)
    const stripped = content
      // Remove type annotations like `: Fig.Spec`, `: Fig.Subcommand`, etc.
      .replace(/:\s*Fig\.\w+(\[\])?/g, "")
      // Remove `satisfies Fig.Spec` etc.
      .replace(/satisfies\s+Fig\.\w+/g, "")
      // Remove import statements
      .replace(/^import\s+.*$/gm, "")
      // Remove export type statements
      .replace(/^export\s+type\s+.*$/gm, "")
      // Convert `export default` to assignment
      .replace(/export\s+default/, "var __spec__ =")
      // Handle `const completionSpec = ...` pattern
      .replace(/const\s+completionSpec\s*=/, "var __spec__ =");

    // Wrap in a function to extract the value
    const wrapped = `${stripped}\n;(__spec__ ?? completionSpec)`;

    try {
      specObj = eval(wrapped);
    } catch {
      // If eval fails, skip this spec (it likely uses generators or dynamic features)
      return false;
    }

    if (!specObj || typeof specObj !== "object") {
      return false;
    }

    // Clean the spec: remove function values and unsupported dynamic hooks.
    const cleaned = cleanSpec(specObj);
    if (!cleaned) return false;

    const filename = inputPath.split("/").pop()!.replace(/\.ts$/, ".json");
    const outputPath = `${outputDir}/${filename}`;
    await Deno.writeTextFile(outputPath, JSON.stringify(cleaned, null, 2));
    return true;
  } catch {
    return false;
  }
}

function cleanSpec(obj: unknown): unknown {
  if (obj === null || obj === undefined) return undefined;
  if (typeof obj === "string" || typeof obj === "number" || typeof obj === "boolean") {
    return obj;
  }
  if (typeof obj === "function") {
    // Skip generator/function values — they can't be serialized
    return undefined;
  }
  if (Array.isArray(obj)) {
    const cleaned = obj.map(cleanSpec).filter((x) => x !== undefined);
    return cleaned.length > 0 ? cleaned : undefined;
  }
  if (typeof obj === "object") {
    const result: Record<string, unknown> = {};
    let hasKeys = false;
    for (const [key, value] of Object.entries(obj as Record<string, unknown>)) {
      // Skip unsupported dynamic hooks that require executing arbitrary JS.
      if (key === "custom" || key === "getQueryTerm") {
        continue;
      }
      const cleaned = cleanSpec(value);
      if (cleaned !== undefined) {
        result[key] = cleaned;
        hasKeys = true;
      }
    }
    return hasKeys ? result : undefined;
  }
  return undefined;
}

// Main
const [srcDir, outputDir] = Deno.args;

if (!srcDir || !outputDir) {
  console.log("Usage: convert_specs.ts <fig-autocomplete-src-dir> <output-dir>");
  console.log("");
  console.log("Example:");
  console.log("  git clone https://github.com/withfig/autocomplete /tmp/autocomplete");
  console.log("  deno run --allow-read --allow-write tools/convert_specs.ts /tmp/autocomplete/src specs/");
  Deno.exit(1);
}

await Deno.mkdir(outputDir, { recursive: true });

let converted = 0;
let skipped = 0;

for (const cmd of PRIORITY_COMMANDS) {
  const inputPath = `${srcDir}/${cmd}.ts`;
  try {
    await Deno.stat(inputPath);
  } catch {
    console.log(`  skip: ${cmd}.ts (not found)`);
    skipped++;
    continue;
  }

  const ok = await convertSpec(inputPath, outputDir);
  if (ok) {
    console.log(`  done: ${cmd}.json`);
    converted++;
  } else {
    console.log(`  skip: ${cmd}.ts (parse failed)`);
    skipped++;
  }
}

console.log(`\nConverted ${converted} specs, skipped ${skipped}`);
