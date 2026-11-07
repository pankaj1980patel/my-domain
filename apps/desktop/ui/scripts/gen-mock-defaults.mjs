// Writes src/dev/mockDefaults.json: for every command in the generated
// bindings (src/bindings.ts), an empty-but-well-formed answer built from its
// return type (strings "", numbers 0, lists [], optional fields left out,
// unions their first choice). The screenshot mock (src/dev/mockTauri.ts)
// answers with these unless it has a sample of its own, so no screen trips
// over a missing field. Rerun after the bindings change:
//
//   node scripts/gen-mock-defaults.mjs
//
// Uses the TypeScript compiler already in devDependencies.

import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = fileURLToPath(new URL("..", import.meta.url));
const file = `${root}src/bindings.ts`;
const program = ts.createProgram([file], { target: ts.ScriptTarget.ES2021, module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Bundler, strict: true, skipLibCheck: true });
const checker = program.getTypeChecker();
const source = program.getSourceFile(file);

function value(type, depth = 0) {
  if (depth > 8) return null;
  const f = type.flags;
  if (f & ts.TypeFlags.Null || f & ts.TypeFlags.Void || f & ts.TypeFlags.Undefined) return null;
  if (f & ts.TypeFlags.StringLiteral) return type.value;
  if (f & ts.TypeFlags.NumberLiteral) return type.value;
  if (f & ts.TypeFlags.BooleanLiteral) return checker.typeToString(type) === "true";
  if (f & ts.TypeFlags.String) return "";
  if (f & ts.TypeFlags.Number) return 0;
  if (f & ts.TypeFlags.Boolean) return false;
  if (f & ts.TypeFlags.Any || f & ts.TypeFlags.Unknown) return null;
  if (type.isUnion()) {
    const parts = type.types.filter((t) => !(t.flags & (ts.TypeFlags.Null | ts.TypeFlags.Undefined)));
    // Nullable: null is the honest empty answer.
    if (parts.length < type.types.length) return null;
    return value(parts[0], depth + 1);
  }
  if (checker.isArrayType(type) || checker.isTupleType(type)) return [];
  if (f & ts.TypeFlags.Object) {
    const out = {};
    for (const p of checker.getPropertiesOfType(type)) {
      if (p.flags & ts.SymbolFlags.Optional) continue;
      out[p.name] = value(checker.getTypeOfSymbolAtLocation(p, source), depth + 1);
    }
    // A string index signature ({ [key in string]: X }): an empty map.
    return out;
  }
  return null;
}

const out = {};
ts.forEachChild(source, function visit(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === "commands" && node.initializer) {
    for (const prop of node.initializer.properties) {
      const text = prop.initializer.getText(source);
      const cmd = /__TAURI_INVOKE(?:<[^(]*>)?\("([a-z0-9_]+)"/.exec(text)?.[1];
      if (!cmd) continue;
      const sig = checker.getSignaturesOfType(checker.getTypeAtLocation(prop.initializer), ts.SignatureKind.Call)[0];
      let ret = checker.getAwaitedType(sig.getReturnType()) ?? sig.getReturnType();
      // typedError<T, E>: the command itself answers T.
      if (/=>\s*typedError/.test(text) && ret.isUnion()) {
        const ok = ret.types.find((t) => checker.getPropertyOfType(t, "data"));
        if (ok) ret = checker.getTypeOfSymbolAtLocation(checker.getPropertyOfType(ok, "data"), source);
      }
      out[cmd] = value(ret);
    }
  }
  ts.forEachChild(node, visit);
});

const target = `${root}src/dev/mockDefaults.json`;
writeFileSync(target, `${JSON.stringify(out, null, 1)}\n`);
console.log(`${Object.keys(out).length} commands -> ${target}`);
