import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

interface PublicModule {
  exports: string[];
  purpose: string;
  kind?: "resources";
}

interface ModuleReference {
  specifier: string | null;
  names: string[] | null;
  position: number;
}

/** Allows UI test adapters to replace named public exports without importing private implementation. */
function mockExports(factory: ts.Expression | undefined): string[] | null {
  if (
    !factory ||
    !ts.isArrowFunction(factory) ||
    factory.parameters.length !== 0
  )
    return null;
  let body = factory.body;
  while (ts.isParenthesizedExpression(body)) body = body.expression;
  if (!ts.isObjectLiteralExpression(body)) return null;
  const names: string[] = [];
  for (const property of body.properties) {
    if (
      ts.isSpreadAssignment(property) ||
      !property.name ||
      ts.isComputedPropertyName(property.name)
    )
      return null;
    if (!ts.isIdentifier(property.name) && !ts.isStringLiteral(property.name))
      return null;
    names.push(property.name.text);
  }
  return names.length ? names : null;
}

/** Finds static and whole-module access, including type-only and test mock imports. */
function references(source: ts.SourceFile): ModuleReference[] {
  const found: ModuleReference[] = [];
  const literal = (node: ts.Node | undefined): string | null =>
    node !== undefined && ts.isStringLiteralLike(node) ? node.text : null;
  const names = (
    elements: ts.NodeArray<ts.ImportSpecifier | ts.ExportSpecifier>,
  ) => elements.map((element) => (element.propertyName ?? element.name).text);
  function visit(node: ts.Node): void {
    if (ts.isImportDeclaration(node)) {
      const clause = node.importClause;
      const bindings = clause?.namedBindings;
      found.push({
        specifier: literal(node.moduleSpecifier),
        names:
          clause === undefined ||
          (bindings !== undefined && ts.isNamespaceImport(bindings))
            ? null
            : [
                ...(clause.name ? ["default"] : []),
                ...(bindings && ts.isNamedImports(bindings)
                  ? names(bindings.elements)
                  : []),
              ],
        position: node.getStart(source),
      });
    } else if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
      found.push({
        specifier: literal(node.moduleSpecifier),
        names:
          node.exportClause && ts.isNamedExports(node.exportClause)
            ? names(node.exportClause.elements)
            : null,
        position: node.getStart(source),
      });
    } else if (ts.isImportTypeNode(node)) {
      const qualifier = node.qualifier;
      let head = qualifier;
      while (head && ts.isQualifiedName(head)) head = head.left;
      found.push({
        specifier: ts.isLiteralTypeNode(node.argument)
          ? literal(node.argument.literal)
          : null,
        names: head && ts.isIdentifier(head) ? [head.text] : null,
        position: node.getStart(source),
      });
    } else if (
      ts.isImportEqualsDeclaration(node) &&
      ts.isExternalModuleReference(node.moduleReference)
    ) {
      found.push({
        specifier: literal(node.moduleReference.expression),
        names: null,
        position: node.getStart(source),
      });
    } else if (ts.isCallExpression(node)) {
      const expression = node.expression;
      const isImport = expression.kind === ts.SyntaxKind.ImportKeyword;
      const isRequire =
        ts.isIdentifier(expression) && expression.text === "require";
      const isMock =
        ts.isPropertyAccessExpression(expression) &&
        ts.isIdentifier(expression.expression) &&
        ["vi", "jest"].includes(expression.expression.text) &&
        [
          "mock",
          "doMock",
          "unmock",
          "doUnmock",
          "importActual",
          "importMock",
        ].includes(expression.name.text);
      if (isImport || isRequire || isMock) {
        found.push({
          specifier: literal(node.arguments[0]),
          names:
            isMock &&
            ts.isPropertyAccessExpression(expression) &&
            ["mock", "doMock"].includes(expression.name.text)
              ? mockExports(node.arguments[1])
              : null,
          position: node.getStart(source),
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  return found;
}

/** Walks source trees without traversing dependency, build, or nested Git metadata. */
function sourceFiles(directory: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (
      entry.isDirectory() &&
      !["node_modules", "dist", "target", ".git"].includes(entry.name)
    ) {
      files.push(...sourceFiles(filename));
    } else if (entry.isFile() && /\.[cm]?tsx?$/.test(entry.name)) {
      files.push(filename);
    }
  }
  return files;
}

/** Checks named public interfaces without adding runtime barrels or weakening TypeScript checking. */
export function checkFeatureInterfaces(root: string): string[] {
  const shell = path.join(root, "packages", "app-shell");
  const featureRoot = path.join(shell, "src", "features");
  const configPath = path.join(shell, "tsconfig.json");
  const config = ts.readConfigFile(configPath, ts.sys.readFile);
  if (config.error)
    throw new Error(
      ts.flattenDiagnosticMessageText(config.error.messageText, "\n"),
    );
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, shell);
  if (parsed.errors.length)
    throw new Error(
      parsed.errors
        .map((error) =>
          ts.flattenDiagnosticMessageText(error.messageText, "\n"),
        )
        .join("\n"),
    );
  const files = ["apps", "packages"].flatMap((directory) =>
    sourceFiles(path.join(root, directory)),
  );
  const program = ts.createProgram(files, parsed.options);
  const checker = program.getTypeChecker();
  const errors: string[] = [];
  const interfaces = new Map<string, PublicModule>();
  const featureNames = new Set(
    readdirSync(featureRoot, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name),
  );
  const owner = (filename: string): string | undefined => {
    const relative = path.relative(featureRoot, filename);
    const name = relative.split(path.sep)[0];
    return !relative.startsWith(`..${path.sep}`) && featureNames.has(name)
      ? name
      : undefined;
  };
  const resolve = (specifier: string, from: string): string | undefined =>
    ts.resolveModuleName(specifier, from, parsed.options, ts.sys).resolvedModule
      ?.resolvedFileName;

  for (const feature of featureNames) {
    const manifest = path.join(featureRoot, feature, "interface.json");
    // A feature without a manifest is private, not implicitly open to consumers.
    if (!ts.sys.fileExists(manifest)) continue;
    const policy = JSON.parse(readFileSync(manifest, "utf8"));
    if (
      typeof policy.owner !== "string" ||
      policy.owner.trim() === "" ||
      !policy.modules ||
      typeof policy.modules !== "object" ||
      Array.isArray(policy.modules)
    ) {
      errors.push(`${manifest}: owner and explicit modules are required`);
      continue;
    }
    for (const [specifier, value] of Object.entries(policy.modules)) {
      const entry = value as PublicModule;
      const target = resolve(
        specifier,
        path.join(featureRoot, feature, "interface.ts"),
      );
      if (
        !specifier.startsWith("./") ||
        specifier.split("/").includes("..") ||
        !target ||
        owner(target) !== feature ||
        !entry ||
        !Array.isArray(entry.exports) ||
        entry.exports.length === 0 ||
        entry.exports.some(
          (name) =>
            typeof name !== "string" || !/^[$A-Z_a-z][$\w]*$/.test(name),
        ) ||
        new Set(entry.exports).size !== entry.exports.length ||
        typeof entry.purpose !== "string" ||
        entry.purpose.trim().length < 20 ||
        (entry.kind !== undefined && entry.kind !== "resources")
      ) {
        errors.push(
          `${manifest}: invalid public module ${specifier}; require local named exports and a meaningful purpose`,
        );
        continue;
      }
      const source = program.getSourceFile(target);
      const symbol = source && checker.getSymbolAtLocation(source);
      const exported = new Set(
        symbol
          ? checker.getExportsOfModule(symbol).map((item) => item.name)
          : [],
      );
      for (const name of entry.exports) {
        if (!exported.has(name))
          errors.push(`${manifest}: ${specifier} no longer exports ${name}`);
      }
      if (interfaces.has(target))
        errors.push(`${manifest}: duplicate public module ${specifier}`);
      interfaces.set(target, entry);
    }
  }

  for (const filename of files) {
    const source = program.getSourceFile(filename);
    if (!source) continue;
    for (const reference of references(source)) {
      const location = `${path.relative(root, filename)}:${source.getLineAndCharacterOfPosition(reference.position).line + 1}`;
      if (reference.specifier === null) {
        // Non-literal module access cannot be checked. Keep it explicit instead of silently bypassing the gate.
        errors.push(`${location}: module access must use a static string`);
        continue;
      }
      const target = resolve(reference.specifier, filename);
      if (
        !target ||
        owner(target) === undefined ||
        owner(target) === owner(filename)
      )
        continue;
      const shellRelative = path.relative(path.join(shell, "src"), filename);
      if (shellRelative.startsWith(`state${path.sep}`)) {
        errors.push(
          `${location}: shared state must not depend on feature UI (${reference.specifier})`,
        );
        continue;
      }
      const entry = interfaces.get(target);
      if (!entry) {
        errors.push(
          `${location}: private feature module ${reference.specifier}`,
        );
      } else if (
        entry.kind === "resources" &&
        filename !== path.join(shell, "src", "i18n", "resources.ts")
      ) {
        errors.push(
          `${location}: feature resources belong to the single i18n composition`,
        );
      } else if (reference.names === null) {
        errors.push(
          `${location}: cross-feature access requires named imports, not whole-module access (${reference.specifier})`,
        );
      } else {
        for (const name of reference.names) {
          if (!entry.exports.includes(name))
            errors.push(
              `${location}: private feature export ${name} from ${reference.specifier}`,
            );
        }
      }
    }
  }
  return errors.sort();
}

if (import.meta.main) {
  const errors = checkFeatureInterfaces(
    fileURLToPath(new URL("../", import.meta.url)),
  );
  if (errors.length) throw new Error(errors.join("\n"));
  console.log(
    "Feature interfaces: named public exports and shared-state direction verified.",
  );
}
