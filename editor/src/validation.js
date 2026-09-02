const issue = (severity, fieldPath, message, modelKey) => ({
  severity,
  fieldPath,
  message,
  ...(modelKey ? { modelKey } : {}),
});

const present = (value) => value !== undefined && value !== null;

export function validateDocument(document) {
  const issues = [];
  const file = document?.file ?? {};
  const materials = Array.isArray(document?.materials) ? document.materials : [];
  const models = Array.isArray(document?.models) ? document.models : [];

  if (document?.type !== "gm.file/1") {
    issues.push(issue("error", "type", "expected a 'gm.file/1' document"));
  }
  if (!String(file.name ?? "").trim()) {
    issues.push(issue("error", "file.name", "file name is empty"));
  }
  if (!present(file.verticalDatum)) {
    issues.push(issue("warning", "file.verticalDatum", "no vertical datum declared; every level in this file is ambiguous"));
  }
  if (!present(file.crs) && models.some((model) => present(model.x) || present(model.y))) {
    issues.push(issue("warning", "file.crs", "models carry coordinates but the file declares no CRS"));
  }

  const materialKeys = new Set();
  for (const [index, material] of materials.entries()) {
    const key = String(material.materialKey ?? "");
    if (!key.trim()) issues.push(issue("error", `materials[${index}].materialKey`, "material key is empty"));
    if (materialKeys.has(key)) issues.push(issue("error", `materials[${index}].materialKey`, `duplicate material key '${key}'`));
    materialKeys.add(key);

    const path = `materials.${key || index}`;
    if (!material.properties?.unitWeight) {
      issues.push(issue("warning", `${path}.properties.unitWeight`, `material '${key}' has no unit weight; self-weight stress cannot be computed`));
    } else if (present(material.properties.unitWeight.value)) {
      const value = Number(material.properties.unitWeight.value);
      if (value < 10 || value > 30) {
        issues.push(issue("warning", `${path}.properties.unitWeight`, `unit weight of ${value} kN/m3 is outside the usual 10–30 range for soil and rock`));
      }
    }

    for (const [propertyKey, bounded] of Object.entries(material.properties ?? {})) {
      if (present(bounded.lower) && present(bounded.upper) && Number(bounded.lower) > Number(bounded.upper)) {
        issues.push(issue("error", `${path}.properties.${propertyKey}`, "lower bound is greater than upper bound"));
      }
      if (present(bounded.value) && present(bounded.lower) && Number(bounded.value) < Number(bounded.lower)) {
        issues.push(issue("warning", `${path}.properties.${propertyKey}.value`, "value is below its lower bound"));
      }
      if (present(bounded.value) && present(bounded.upper) && Number(bounded.value) > Number(bounded.upper)) {
        issues.push(issue("warning", `${path}.properties.${propertyKey}.value`, "value is above its upper bound"));
      }
    }
    if (!(material.constitutiveModels ?? []).length) {
      issues.push(issue("warning", `${path}.constitutiveModels`, `material '${key}' has no constitutive model, so it cannot be analysed`));
    }
  }

  const modelKeys = new Set();
  const usedMaterials = new Set();
  for (const [modelIndex, model] of models.entries()) {
    const key = String(model.modelKey ?? "");
    if (!key.trim()) issues.push(issue("error", `models[${modelIndex}].modelKey`, "model key is empty", key));
    if (modelKeys.has(key)) issues.push(issue("error", `models[${modelIndex}].modelKey`, `duplicate model key '${key}'`, key));
    modelKeys.add(key);
    const layers = Array.isArray(model.layers) ? model.layers : [];
    const gammaW = Number(model.gammaW ?? 9.81);
    if (gammaW < 9 || gammaW > 11) {
      issues.push(issue("warning", "gammaW", `unit weight of water is ${gammaW} kN/m3, outside the plausible 9–11 range`, key));
    }
    if (!layers.length) {
      issues.push(issue("warning", "layers", "model has no layers", key));
      continue;
    }
    for (let index = 0; index < layers.length; index += 1) {
      const layer = layers[index];
      usedMaterials.add(layer.materialKey);
      if (!materialKeys.has(layer.materialKey)) {
        issues.push(issue("error", `layers[${index}].materialKey`, `references material '${layer.materialKey}', which the file does not define`, key));
      }
      if (!Number.isFinite(Number(layer.topLevel))) {
        issues.push(issue("error", `layers[${index}].topLevel`, "top level is not a finite number", key));
      }
      if (index > 0 && Number(layer.topLevel) >= Number(layers[index - 1].topLevel)) {
        const relation = Number(layer.topLevel) === Number(layers[index - 1].topLevel)
          ? "gives a zero-thickness stratum"
          : "layers must be ordered downwards";
        issues.push(issue("error", `layers[${index}].topLevel`, `layer ${index + 1} starts at ${layer.topLevel}; ${relation}`, key));
      }
    }
    const firstTop = Number(layers[0].topLevel);
    const lastTop = Number(layers.at(-1).topLevel);
    if (!present(model.surfaceLevel)) {
      issues.push(issue("warning", "surfaceLevel", "no surface level; depths below ground cannot be computed", key));
    } else if (Number(model.surfaceLevel) !== firstTop) {
      issues.push(issue(model.settings?.allowGaps ? "warning" : "error", "surfaceLevel", `surface level ${model.surfaceLevel} does not meet the first layer at ${firstTop}`, key));
    }
    if (!present(model.baseLevel)) {
      issues.push(issue("warning", "baseLevel", "no base level; the deepest layer has no bottom", key));
    } else if (Number(model.baseLevel) >= lastTop) {
      issues.push(issue("error", "baseLevel", `base level ${model.baseLevel} is not below the deepest layer at ${lastTop}`, key));
    }
    const groundwater = model.groundwater ?? { kind: "unknown" };
    if (groundwater.kind === "unknown") {
      issues.push(issue("warning", "groundwater", "groundwater regime is unknown; consumers cannot compute effective stress", key));
    } else if (groundwater.kind === "hydrostatic") {
      const depth = Number(groundwater.depth);
      if (depth < 0) issues.push(issue("warning", "groundwater.depth", `water table is ${-depth} m above ground level`, key));
      if (present(model.surfaceLevel) && present(model.baseLevel) && Number(model.surfaceLevel) - depth < Number(model.baseLevel)) {
        issues.push(issue("warning", "groundwater.depth", "water table is below the base; the model is entirely dry", key));
      }
    }
  }

  for (const key of materialKeys) {
    if (!usedMaterials.has(key)) issues.push(issue("warning", `materials.${key}`, `material '${key}' is not used by any model`));
  }
  return issues;
}

export function issueCounts(issues) {
  return issues.reduce((counts, item) => {
    counts[item.severity] += 1;
    return counts;
  }, { error: 0, warning: 0 });
}
