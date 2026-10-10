/**
 * Build utilities for pglet tests.
 *
 * Provides functions to build various targets (native, WASM) on demand.
 */

/**
 * Build pglet_server (debug build).
 */
export async function buildServer(): Promise<boolean> {
  console.log("[build] cargo build -p pglet_server...");
  const cmd = new Deno.Command("cargo", {
    args: ["build", "-p", "pglet_server"],
    stdout: "inherit",
    stderr: "inherit",
  });
  const result = await cmd.output();
  if (!result.success) {
    console.error("[build] Failed to build pglet_server");
  }
  return result.success;
}

/**
 * A wasm-pack build of `crates/pglet_wasm`: its output directory, and the cargo features
 * it adds to the crate's defaults.
 */
export interface WasmPackage {
  out_dir: string;
  features: string[];
}

/** The default build — the shipped artifact's shape (`deno task wasm:build`). */
export const WASM_DEFAULT: WasmPackage = { out_dir: "pkg", features: [] };

/**
 * The build with the `opfs` feature on, which alone exports `PgletBtreePersistent`
 * (`deno task wasm:build:opfs`).
 */
export const WASM_OPFS: WasmPackage = { out_dir: "pkg-opfs", features: ["opfs"] };

const wasm_js_path = (pkg: WasmPackage): string =>
  `crates/pglet_wasm/${pkg.out_dir}/pglet_wasm.js`;

/**
 * Build a WASM package with wasm-pack.
 */
export async function buildWasm(pkg: WasmPackage = WASM_DEFAULT): Promise<boolean> {
  const args = ["build", "crates/pglet_wasm", "--target", "web", "--out-dir", pkg.out_dir];
  if (pkg.features.length > 0) args.push("--features", pkg.features.join(","));
  console.log(`[build] wasm-pack ${args.join(" ")}...`);
  const cmd = new Deno.Command("wasm-pack", {
    args,
    stdout: "inherit",
    stderr: "inherit",
  });
  const result = await cmd.output();
  if (!result.success) {
    console.error(`[build] Failed to build WASM package ${pkg.out_dir}`);
  }
  return result.success;
}

/**
 * Check if a WASM package exists.
 */
export async function wasmPackageExists(pkg: WasmPackage = WASM_DEFAULT): Promise<boolean> {
  try {
    await Deno.stat(wasm_js_path(pkg));
    return true;
  } catch {
    return false;
  }
}

/**
 * Get the mtime of a WASM package.
 */
async function getWasmPackageMtime(pkg: WasmPackage): Promise<number | null> {
  try {
    const stat = await Deno.stat(wasm_js_path(pkg));
    return stat.mtime?.getTime() ?? null;
  } catch {
    return null;
  }
}

/**
 * Find the newest .rs file mtime in the given directories.
 */
async function getNewestRustMtime(dirs: string[]): Promise<number> {
  let newest = 0;

  for (const dir of dirs) {
    for await (const entry of Deno.readDir(dir)) {
      const path = `${dir}/${entry.name}`;
      if (entry.isDirectory) {
        const subNewest = await getNewestRustMtime([path]);
        if (subNewest > newest) newest = subNewest;
      } else if (entry.name.endsWith(".rs")) {
        const stat = await Deno.stat(path);
        const mtime = stat.mtime?.getTime() ?? 0;
        if (mtime > newest) newest = mtime;
      }
    }
  }

  return newest;
}

/**
 * Check if a WASM package is stale (source newer than build).
 */
async function isWasmStale(pkg: WasmPackage): Promise<boolean> {
  const pkgMtime = await getWasmPackageMtime(pkg);
  if (pkgMtime === null) return true; // No package = stale

  // Every crate the wasm build links
  const sourceDirs = [
    "crates/pglet_wasm/src",
    "crates/pglet/src",
    "crates/pglet_core/src",
    "crates/pglet_sql/src",
    "crates/pglet_storage/src",
  ];

  const newestSource = await getNewestRustMtime(sourceDirs);
  return newestSource > pkgMtime;
}

/**
 * Ensure a WASM package is built and up-to-date.
 * Rebuilds if missing or stale (source newer than build).
 */
export async function ensureWasm(pkg: WasmPackage = WASM_DEFAULT): Promise<void> {
  if (await wasmPackageExists(pkg) && !await isWasmStale(pkg)) {
    console.log(`[build] WASM package ${pkg.out_dir} already exists`);
    return;
  }

  const reason = await wasmPackageExists(pkg) ? "stale" : "not found";
  console.log(`[build] WASM package ${pkg.out_dir} ${reason}, building...`);
  if (!await buildWasm(pkg)) {
    throw new Error(
      `Failed to build WASM package ${pkg.out_dir}\n` +
      `Fix: deno task ${pkg.features.length > 0 ? "wasm:build:opfs" : "wasm:build"}`
    );
  }
}
