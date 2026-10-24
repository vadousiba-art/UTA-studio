// Build the isolated plugin helper (uta-plugin-host) and place it where
// Tauri expects an external binary: src-tauri/binaries/uta-plugin-host-<target triple>[.exe].
// Tauri copies it next to the app executable in dev builds and installers.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const cargo = process.env.CARGO || "cargo";
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  execFileSync("rustc", ["-vV"], { encoding: "utf8" })
    .split("\n")
    .find((l) => l.startsWith("host:"))
    .slice(5)
    .trim();
const exe = triple.includes("windows") ? ".exe" : "";
const args = ["build", "--release", "-p", "uta-plugin-host"];
if (process.env.TAURI_ENV_TARGET_TRIPLE) args.push("--target", triple);
execFileSync(cargo, args, { cwd: root, stdio: "inherit" });
const built = process.env.TAURI_ENV_TARGET_TRIPLE
  ? join(root, "target", triple, "release", `uta-plugin-host${exe}`)
  : join(root, "target", "release", `uta-plugin-host${exe}`);
const dest = join(root, "src-tauri", "binaries", `uta-plugin-host-${triple}${exe}`);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(built, dest);
console.log(`plugin helper ready: ${dest}`);
