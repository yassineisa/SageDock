/**
 * The single icon family for the whole app: **Segoe Fluent Icons**.
 *
 * Chosen because it is Microsoft's own UI icon font and ships with Windows 11, so SageDock
 * bundles no font files, adds nothing to a 1.4 GB installer, needs no `font-src` in the
 * CSP, and carries no third-party licence obligation. Windows 10 falls back to Segoe MDL2
 * Assets, which shares most of these codepoints.
 *
 * Every glyph is referenced through this map rather than inline escapes. The codepoints are
 * private-use characters, so a wrong one renders as an empty box rather than failing
 * loudly — keeping them in one verified table is what stops that spreading.
 *
 * Icons are always decorative here: each is `aria-hidden`, and every control that uses one
 * carries its own visible text or `aria-label`. That also keeps icon glyphs out of
 * accessible names, which the UI tests match on exactly.
 */
const GLYPHS = {
  home: "",
  settings: "",
  help: "",
  repair: "",
  refresh: "",
  folder: "",
  folderOpen: "",
  play: "",
  stop: "",
  add: "",
  rename: "",
  edit: "",
  remove: "",
  save: "",
  openFile: "",
  search: "",
  document: "",
  warning: "",
  error: "",
  success: "",
  accept: "",
  info: "",
  chevronRight: "",
  diagnostic: "",
  calculator: "",
  // Written as escapes rather than pasted glyphs. These four were added later, and a
  // private-use codepoint that silently degrades to an empty box during a copy or an editor
  // round-trip is exactly the failure this table exists to prevent; an escape cannot.
  // Each was verified to render a real, distinct glyph in Segoe Fluent Icons rather than
  // .notdef, and checked at 16px in both themes. Meanings: braces for source code, a ruler
  // and set square for numerical engineering work, a wrench and screwdriver for build
  // tools, and a toolbox for the complete kit.
  code: "",
  numeric: "",
  buildTools: "",
  toolkit: "",
} as const;

export type IconName = keyof typeof GLYPHS;

export function Icon({
  name,
  size = 16,
  className,
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  return (
    <span
      aria-hidden="true"
      className={className ? `icon ${className}` : "icon"}
      style={{
        fontSize: `${size}px`,
        width: `${size}px`,
        height: `${size}px`,
        lineHeight: `${size}px`,
      }}
    >
      {GLYPHS[name]}
    </span>
  );
}
