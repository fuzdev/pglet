#!/usr/bin/env -S deno run --allow-net --allow-read

// A static file server over the repo root, so the example pages can load the wasm
// bindings from `crates/pglet_wasm/pkg/` and `crates/pglet_wasm/pkg-opfs/`. Plain
// `Deno.serve` — no dependency.

const port = 8001;
const root = new URL("../../", import.meta.url);

const CONTENT_TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".css": "text/css; charset=utf-8",
};

function contentType(path: string): string {
  const dot = path.lastIndexOf(".");
  return (dot >= 0 && CONTENT_TYPES[path.slice(dot)]) || "application/octet-stream";
}

console.log(`Starting server on http://localhost:${port}`);
console.log(`Open http://localhost:${port}/examples/browser/ in your browser`);

Deno.serve({ port }, async (req) => {
  let path = decodeURIComponent(new URL(req.url).pathname);
  if (path.endsWith("/")) path += "index.html";
  // `new URL` resolves `..` segments, so a path cannot climb above the root.
  const file = new URL("." + path, root);
  if (!file.href.startsWith(root.href)) {
    return new Response("Forbidden", { status: 403 });
  }
  try {
    const body = await Deno.readFile(file);
    console.log(`${req.method} ${path} 200`);
    return new Response(body, { headers: { "Content-Type": contentType(path) } });
  } catch {
    console.log(`${req.method} ${path} 404`);
    return new Response("Not Found", { status: 404 });
  }
});
