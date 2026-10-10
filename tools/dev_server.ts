#!/usr/bin/env -S deno run --allow-read=crates --allow-run=cargo
/**
 * Development server with auto-restart on .rs file changes.
 *
 * Usage: deno task server
 *
 * Watches crates/ for .rs changes, debounces, rebuilds, and restarts. `deno task server:restart`
 * stops the server; the next `.rs` save rebuilds and starts it again.
 */

const DEBOUNCE_MS = 500;

let serverProcess: Deno.ChildProcess | null = null;
let restartTimeout: number | null = null;
let isRestarting = false;

// Graceful shutdown on Ctrl+C
Deno.addSignalListener("SIGINT", async () => {
  console.log("\n\x1b[36m[server]\x1b[0m Shutting down...");
  if (serverProcess) {
    try {
      serverProcess.kill("SIGTERM");
      await serverProcess.status;
    } catch { /* already dead */ }
  }
  Deno.exit(0);
});

async function build(): Promise<boolean> {
  console.log("\x1b[36m[server]\x1b[0m Building...");
  const cmd = new Deno.Command("cargo", {
    args: ["build"],
    stdout: "inherit",
    stderr: "inherit",
  });
  const status = await cmd.output();
  return status.success;
}

function startServer(): Deno.ChildProcess {
  console.log("\x1b[36m[server]\x1b[0m Starting pglet_server on :5433");
  const cmd = new Deno.Command("cargo", {
    args: ["run", "-p", "pglet_server"],
    stdout: "inherit",
    stderr: "inherit",
  });
  return cmd.spawn();
}

async function restart() {
  if (isRestarting) return;
  isRestarting = true;

  try {
    if (serverProcess) {
      console.log("\x1b[36m[server]\x1b[0m Stopping server...");
      try {
        serverProcess.kill("SIGTERM");
        await serverProcess.status;
      } catch {
        // Process already dead
      }
      serverProcess = null;
    }

    if (await build()) {
      serverProcess = startServer();
      // Monitor for unexpected exits
      serverProcess.status.then((status) => {
        if (!isRestarting && serverProcess) {
          console.log(`\x1b[33m[server]\x1b[0m Server exited with code ${status.code}`);
          serverProcess = null;
        }
      });
    } else {
      console.log("\x1b[31m[server]\x1b[0m Build failed, waiting for changes...");
    }
  } finally {
    isRestarting = false;
  }
}

function scheduleRestart() {
  if (restartTimeout) {
    clearTimeout(restartTimeout);
  }
  restartTimeout = setTimeout(() => {
    restartTimeout = null;
    restart();
  }, DEBOUNCE_MS);
}

// Initial start
await restart();

// Watch for .rs file changes
console.log("\x1b[36m[server]\x1b[0m Watching crates/**/*.rs for changes...");
const watcher = Deno.watchFs("crates", { recursive: true });

for await (const event of watcher) {
  const hasRsChanges = event.paths.some((p) => p.endsWith(".rs"));
  if (hasRsChanges && (event.kind === "modify" || event.kind === "create")) {
    console.log(`\x1b[36m[server]\x1b[0m Change detected: ${event.paths[0]}`);
    scheduleRestart();
  }
}
