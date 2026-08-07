import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { WASI } from "node:wasi";

const [, , wasmArgument, configArgument, libraryArgument] = process.argv;

if (!wasmArgument || !configArgument || !libraryArgument) {
  console.error("Usage: node run-wasi.mjs <module.wasm> <config.json> <library-root>");
  process.exit(2);
}

const wasmPath = resolve(wasmArgument);
const configPath = resolve(configArgument);
const libraryRoot = resolve(libraryArgument);

for (const [label, path] of [
  ["WASM module", wasmPath],
  ["configuration", configPath],
  ["library root", libraryRoot],
]) {
  if (!existsSync(path)) {
    console.error(`${label} does not exist: ${path}`);
    process.exit(2);
  }
}

const wasi = new WASI({
  version: "preview1",
  args: ["altium-designer-mcp", "/config/config.json"],
  env: {
    HOME: "/config",
    RUST_LOG: process.env.RUST_LOG ?? "warn",
  },
  preopens: {
    "/config": dirname(configPath),
    "/libraries": libraryRoot,
  },
});

const module = await WebAssembly.compile(readFileSync(wasmPath));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
wasi.start(instance);
