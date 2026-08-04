import {
  IconBrandGolang,
  IconBrandPython,
  IconBrandSvelte,
  IconBraces,
  IconFileCode,
  IconFileText,
  IconFileTypeCss,
  IconFileTypeHtml,
  IconFileTypeJs,
  IconFileTypeJsx,
  IconFileTypeRs,
  IconFileTypeSql,
  IconFileTypeSvg,
  IconFileTypeTs,
  IconFileTypeTsx,
  IconFileTypeVue,
  IconFileTypeXml,
  IconMarkdown,
  IconPhoto,
  IconSettings,
} from "@tabler/icons-react";

export interface WorkspaceFileVisual {
  Icon: typeof IconFileCode;
  iconClassName: string;
  language: string;
  label: string;
}

const DEFAULT_VISUAL: WorkspaceFileVisual = {
  Icon: IconFileText,
  iconClassName: "text-muted-foreground",
  language: "text",
  label: "TEXT",
};

const CODE_VISUAL: WorkspaceFileVisual = {
  Icon: IconFileCode,
  iconClassName: "text-slate-500 dark:text-slate-400",
  language: "text",
  label: "CODE",
};

const VISUALS_BY_EXTENSION: Readonly<Record<string, WorkspaceFileVisual>> = {
  c: { Icon: IconFileCode, iconClassName: "text-blue-600 dark:text-blue-400", language: "c", label: "C" },
  cc: { Icon: IconFileCode, iconClassName: "text-blue-600 dark:text-blue-400", language: "cpp", label: "C++" },
  cpp: { Icon: IconFileCode, iconClassName: "text-blue-600 dark:text-blue-400", language: "cpp", label: "C++" },
  css: { Icon: IconFileTypeCss, iconClassName: "text-blue-600 dark:text-blue-400", language: "css", label: "CSS" },
  go: { Icon: IconBrandGolang, iconClassName: "text-cyan-600 dark:text-cyan-400", language: "go", label: "GO" },
  h: { Icon: IconFileCode, iconClassName: "text-violet-600 dark:text-violet-400", language: "c", label: "C" },
  hpp: { Icon: IconFileCode, iconClassName: "text-violet-600 dark:text-violet-400", language: "cpp", label: "C++" },
  html: { Icon: IconFileTypeHtml, iconClassName: "text-orange-600 dark:text-orange-400", language: "html", label: "HTML" },
  js: { Icon: IconFileTypeJs, iconClassName: "text-amber-600 dark:text-amber-400", language: "javascript", label: "JS" },
  json: { Icon: IconBraces, iconClassName: "text-amber-600 dark:text-amber-400", language: "json", label: "JSON" },
  jsonc: { Icon: IconBraces, iconClassName: "text-amber-600 dark:text-amber-400", language: "jsonc", label: "JSONC" },
  jsx: { Icon: IconFileTypeJsx, iconClassName: "text-amber-600 dark:text-amber-400", language: "jsx", label: "JSX" },
  md: { Icon: IconMarkdown, iconClassName: "text-sky-600 dark:text-sky-400", language: "markdown", label: "MD" },
  mdx: { Icon: IconMarkdown, iconClassName: "text-sky-600 dark:text-sky-400", language: "mdx", label: "MDX" },
  mjs: { Icon: IconFileTypeJs, iconClassName: "text-amber-600 dark:text-amber-400", language: "javascript", label: "JS" },
  py: { Icon: IconBrandPython, iconClassName: "text-blue-600 dark:text-blue-400", language: "python", label: "PY" },
  rs: { Icon: IconFileTypeRs, iconClassName: "text-orange-700 dark:text-orange-400", language: "rust", label: "RUST" },
  sass: { Icon: IconFileTypeCss, iconClassName: "text-pink-600 dark:text-pink-400", language: "sass", label: "SASS" },
  scss: { Icon: IconFileTypeCss, iconClassName: "text-pink-600 dark:text-pink-400", language: "scss", label: "SCSS" },
  sql: { Icon: IconFileTypeSql, iconClassName: "text-violet-600 dark:text-violet-400", language: "sql", label: "SQL" },
  svelte: { Icon: IconBrandSvelte, iconClassName: "text-orange-600 dark:text-orange-400", language: "svelte", label: "SVELTE" },
  svg: { Icon: IconFileTypeSvg, iconClassName: "text-emerald-600 dark:text-emerald-400", language: "xml", label: "SVG" },
  toml: { Icon: IconSettings, iconClassName: "text-slate-600 dark:text-slate-400", language: "toml", label: "TOML" },
  ts: { Icon: IconFileTypeTs, iconClassName: "text-blue-600 dark:text-blue-400", language: "typescript", label: "TS" },
  tsx: { Icon: IconFileTypeTsx, iconClassName: "text-cyan-600 dark:text-cyan-400", language: "tsx", label: "TSX" },
  vue: { Icon: IconFileTypeVue, iconClassName: "text-emerald-600 dark:text-emerald-400", language: "vue", label: "VUE" },
  xml: { Icon: IconFileTypeXml, iconClassName: "text-orange-600 dark:text-orange-400", language: "xml", label: "XML" },
  yaml: { Icon: IconSettings, iconClassName: "text-violet-600 dark:text-violet-400", language: "yaml", label: "YAML" },
  yml: { Icon: IconSettings, iconClassName: "text-violet-600 dark:text-violet-400", language: "yaml", label: "YAML" },
};

const IMAGE_EXTENSIONS = new Set(["avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "webp"]);
const CODE_FILENAMES = new Map<string, WorkspaceFileVisual>([
  ["dockerfile", { Icon: IconFileCode, iconClassName: "text-cyan-600 dark:text-cyan-400", language: "docker", label: "DOCKER" }],
  ["makefile", { Icon: IconFileCode, iconClassName: "text-violet-600 dark:text-violet-400", language: "make", label: "MAKE" }],
]);

/** Returns the shared icon, color, label, and Shiki language for a workspace path. */
// eslint-disable-next-line react-refresh/only-export-components
export function workspaceFileVisual(path: string): WorkspaceFileVisual {
  const filename = path.split(/[\\/]/).at(-1)?.toLowerCase() ?? "";
  const namedVisual = CODE_FILENAMES.get(filename);
  if (namedVisual !== undefined) return namedVisual;

  if (filename === "cargo.lock") return VISUALS_BY_EXTENSION.toml;
  const extension = filename.includes(".") ? filename.split(".").at(-1) ?? "" : "";
  if (IMAGE_EXTENSIONS.has(extension)) {
    return {
      Icon: IconPhoto,
      iconClassName: "text-emerald-600 dark:text-emerald-400",
      language: "text",
      label: extension.toUpperCase(),
    };
  }
  return VISUALS_BY_EXTENSION[extension] ?? (extension === "" ? DEFAULT_VISUAL : CODE_VISUAL);
}

/** Renders the compact file-type glyph used by explorer and search rows. */
export function WorkspaceFileIcon({ path, className = "size-4" }: { path: string; className?: string }) {
  const visual = workspaceFileVisual(path);
  return <visual.Icon aria-hidden="true" className={`${className} shrink-0 ${visual.iconClassName}`} />;
}
