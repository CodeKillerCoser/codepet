// Cross-object selection semantics complement schema shape validation.
// A Provider only returns dimensions it advertises, and fills declared defaults.
export function normalizeSelection(turnSend, input = {}) {
  const result = {};
  const invalid = () => {
    throw Object.assign(Error("Turn selection is invalid or unavailable"), {
      protocolCode: "invalid_selection",
    });
  };
  for (const [field, key] of [
    ["accessMode", "accessModeId"],
    ["reasoningEffort", "reasoningEffortId"],
  ]) {
    const choices = turnSend?.[field];
    if (!choices) {
      if (input[key] !== undefined) invalid();
      continue;
    }
    const value = input[key] ?? choices.defaultId;
    if (
      !choices.options.some(
        (option) => option.id === value && option.enabled !== false,
      )
    )
      invalid();
    result[key] = value;
  }
  const catalog = turnSend?.modelCatalog;
  if (!catalog) {
    if (input.model !== undefined) invalid();
    return result;
  }
  const model = input.model ?? catalog.defaultSelection;
  // This fixture advertises only a flat model catalog. Do not silently accept
  // another model-selection branch or a model outside that catalog.
  if (
    catalog.kind !== "flat" ||
    model?.kind !== "flat" ||
    !catalog.models.some(
      (option) => option.id === model.modelId && option.enabled !== false,
    )
  )
    invalid();
  result.model = { kind: "flat", modelId: model.modelId };
  return result;
}
