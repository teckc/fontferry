import { readFile, writeFile } from "node:fs/promises";

const sourceUrl = new URL("../catalog/source.json", import.meta.url);
const catalogUrl = new URL("../catalog/builtin/catalog.json", import.meta.url);
const { templates, fonts, ...metadata } = JSON.parse(await readFile(sourceUrl, "utf8"));

function escapeRegex(value) {
  return value.replace(/[.*+?^$\{\}()|[\]\\]/g, "\\$&");
}

function expandMatrix(matrix) {
  const variants = matrix.prefixes.flatMap((prefix) => matrix.suffixes.map((suffix) => {
    const packageName = prefix + "-" + suffix;
    const recommendation = matrix.recommended[packageName];
    return {
      id: packageName.toLowerCase(),
      name: recommendation?.name ?? packageName,
      description: recommendation?.description ?? "",
      assetPattern: "^" + escapeRegex(packageName + ".zip") + "$",
      default: packageName === matrix.default,
      advanced: !recommendation,
      webOnly: matrix.webSuffixes.includes(suffix),
    };
  }));
  const packageIds = new Set(variants.map((variant) => variant.id));
  for (const packageName of [matrix.default, ...Object.keys(matrix.recommended)]) {
    if (!packageIds.has(packageName.toLowerCase())) {
      throw new Error("Unknown package in variant matrix " + packageName);
    }
  }
  return variants;
}

function expandTemplate({ name, parameters }) {
  if (!Object.hasOwn(templates, name)) throw new Error("Unknown variant template " + name);
  return templates[name].map((variant) => ({
    ...variant,
    assetPattern: variant.assetPattern.replace(/\{([a-zA-Z]+)\}/g, (_, key) => {
      if (typeof parameters[key] !== "string") throw new Error("Missing template parameter " + key);
      return escapeRegex(parameters[key]);
    }),
  }));
}

function expandFont({ variantMatrix, variantTemplate, ...font }) {
  if (variantMatrix) return { ...font, variants: expandMatrix(variantMatrix) };
  if (variantTemplate) return { ...font, variants: expandTemplate(variantTemplate) };
  return font;
}

const output = JSON.stringify({ ...metadata, fonts: fonts.map(expandFont) }, null, 2) + "\n";
if (process.argv.includes("--check")) {
  const current = (await readFile(catalogUrl, "utf8")).replaceAll("\r\n", "\n");
  if (current !== output) throw new Error("Generated catalog differs. Run node scripts/generate-catalog.mjs.");
} else {
  await writeFile(catalogUrl, output);
}
