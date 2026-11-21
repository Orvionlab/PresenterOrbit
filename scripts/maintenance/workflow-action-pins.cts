const actionReferencePattern = /^\s*(?:-\s*)?uses:\s*([^\s#]+)(?:\s+#.*)?$/gm;
const commitShaPattern = /^[0-9a-f]{40}$/i;

function findUnpinnedWorkflowActions(source, workflowPath) {
  const violations = [];
  for (const match of source.matchAll(actionReferencePattern)) {
    const reference = match[1];
    if (reference.startsWith("./")) continue;
    const separator = reference.lastIndexOf("@");
    const action = separator > 0 ? reference.slice(0, separator) : reference;
    const version = separator > 0 ? reference.slice(separator + 1) : "";
    if (!commitShaPattern.test(version)) {
      const line = source.slice(0, match.index).split(/\r?\n/).length;
      violations.push(`${workflowPath}:${line} ${action} must use an immutable 40-character commit SHA.`);
    }
  }
  return violations;
}

module.exports = { findUnpinnedWorkflowActions };
