export function getMessagePlaceholders(message: string) {
  return [...message.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map((match) => match[1]).sort();
}

export function findDutchTerminologyViolations(messages: Record<string, unknown>) {
  const forbiddenTerms = /\b(?:theme|themes|slide|slides|workspace)\b/i;
  return Object.entries(messages).flatMap(([key, message]) => {
    const searchable = String(message).replace(/\{[a-zA-Z0-9_]+\}/g, "");
    return forbiddenTerms.test(searchable) ? [`nl.${key} contains "${searchable.match(forbiddenTerms)?.[0]}"`] : [];
  });
}

/** Problems between the English and Dutch ARB files: key sets, placeholders, Dutch product terms, metadata debt. */
export function findLocaleProblems(en: Record<string, unknown>, nl: Record<string, unknown>, metadataBaseline: number) {
  const messages = (arb: Record<string, unknown>) => Object.entries(arb).filter(([key]) => !key.startsWith("@"));
  const english = Object.fromEntries(messages(en)) as Record<string, string>;
  const dutch = Object.fromEntries(messages(nl)) as Record<string, string>;
  const problems: string[] = [];
  const englishKeys = Object.keys(english).sort();
  if (JSON.stringify(Object.keys(dutch).sort()) !== JSON.stringify(englishKeys))
    problems.push("Locale key mismatch: nl.arb does not match en.arb.");
  for (const key of englishKeys) {
    if (
      JSON.stringify(getMessagePlaceholders(english[key])) !== JSON.stringify(getMessagePlaceholders(dutch[key] ?? ""))
    )
      problems.push(`Locale placeholder mismatch: nl.${key} does not match en.${key}.`);
  }
  problems.push(...findDutchTerminologyViolations(dutch));
  const described = Object.keys(en).filter((key) => key.startsWith("@") && key !== "@@locale").length;
  if (englishKeys.length - described > metadataBaseline)
    problems.push(
      `English ARB metadata debt increased from ${metadataBaseline} to ${englishKeys.length - described}; every new message needs an @key description.`,
    );
  return problems;
}
