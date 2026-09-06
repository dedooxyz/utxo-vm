import fs from "fs";
import path from "path";
import { execSync } from "child_process";
import crypto from "crypto";

export async function handleCompile(targetFile: string, options: any): Promise<void> {
  console.log(`\n📦 [UTXO-VM Compiler] Compiling AssemblyScript contract: ${targetFile}...`);

  if (!fs.existsSync(targetFile)) {
    console.error(`❌ Error: File not found: ${targetFile}`);
    process.exit(1);
  }

  const outDir = options.outDir || "./build";
  if (!fs.existsSync(outDir)) {
    fs.mkdirSync(outDir, { recursive: true });
  }

  const baseName = path.basename(targetFile, path.extname(targetFile));
  const wasmPath = path.join(outDir, `${baseName}.wasm`);
  const watPath = path.join(outDir, `${baseName}.wat`);

  try {
    const cmd = `npx asc "${targetFile}" --outFile "${wasmPath}" --textFile "${watPath}" --optimizeLevel 3 --shrinkLevel 0 --bindings esm`;
    execSync(cmd, { stdio: "inherit" });

    const wasmBytes = fs.readFileSync(wasmPath);
    const codeHash = crypto.createHash("sha256").update(wasmBytes).digest("hex");

    console.log(`\n✅ Contract compiled successfully!`);
    console.log(`   - WASM Bytecode: ${wasmPath} (${wasmBytes.length} bytes)`);
    console.log(`   - WAT Text:     ${watPath}`);
    console.log(`   - Code Hash:    ${codeHash}`);
  } catch (err: any) {
    console.error(`❌ Compilation failed:`, err.message);
    process.exit(1);
  }
}
