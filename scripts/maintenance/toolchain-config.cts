function parseNpmVersion(userAgent) {
  const match = /\bnpm\/([^\s]+)/.exec(userAgent ?? "");
  return match?.[1];
}

function findRuntimeMismatches({ nodeVersion, npmUserAgent }, expectedNode, expectedNpm) {
  const actualNpm = parseNpmVersion(npmUserAgent);
  const mismatches = [];
  if (nodeVersion !== expectedNode) {
    mismatches.push(["active Node.js runtime", nodeVersion, expectedNode]);
  }
  if (actualNpm !== expectedNpm) {
    mismatches.push(["active npm runtime", actualNpm, expectedNpm]);
  }
  return mismatches;
}

module.exports = { findRuntimeMismatches, parseNpmVersion };
