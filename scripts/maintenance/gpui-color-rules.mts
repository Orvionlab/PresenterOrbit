const gpuiColorPatterns = [
  /\b(?:rgba?|hsla?|opaque_grey|black|white|red|green|blue|yellow|transparent_black|transparent_white)\s*\(/,
  /\b(?:Rgba|Hsla)\s*(?:\{|::)/,
  /\b0x[0-9a-fA-F]{6}(?:[0-9a-fA-F]{2})?\b/,
  /"#[0-9a-fA-F]{3,8}"/,
];

/** Line numbers (1-based) in a Rust source that build a color from a literal instead of reading the theme. */
export function findGpuiColorLiterals(source: string) {
  return source.split(/\r?\n/).flatMap((line, index) => {
    const code = line.replace(/\/\/.*$/, "");
    return gpuiColorPatterns.some((pattern) => pattern.test(code)) ? [index + 1] : [];
  });
}
