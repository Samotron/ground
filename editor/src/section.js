const SVG_NS = "http://www.w3.org/2000/svg";

// FNV-1a over the key's bytes, then a golden-angle multiple so adjacent keys
// land far apart on the colour wheel rather than in neighbouring shades. A
// material therefore keeps its colour on every page, in both this editor and
// `gm ui`.
//
// Take the modulo *before* clamping to 1, matching gm-core. Clamping first
// leaves 0 reachable, which would give a different hue from the Rust
// implementation for roughly one key in 360 — the key "VI" is one.
// assets/conformance.json pins the agreed values.
export function hue(key) {
  let hash = 2166136261;
  for (const byte of new TextEncoder().encode(key)) {
    hash ^= byte;
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return Math.max((Math.imul(hash, 137) >>> 0) % 360, 1);
}

function svgElement(name, attributes = {}, text) {
  const element = document.createElementNS(SVG_NS, name);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, String(value));
  if (text !== undefined) element.textContent = text;
  return element;
}

export function drawSection(model, materials) {
  const surface = Number(model.surfaceLevel);
  const base = Number(model.baseLevel);
  if (!Number.isFinite(surface) || !Number.isFinite(base)) {
    const note = document.createElement("p");
    note.className = "note";
    note.textContent = "Set a surface and base level to draw this section.";
    return note;
  }
  if (!model.layers?.length || base >= surface) {
    const note = document.createElement("p");
    note.className = "note";
    note.textContent = "Nothing coherent to draw yet.";
    return note;
  }

  const width = 460;
  const columnLeft = 92;
  const columnWidth = 132;
  const top = 24;
  const extent = surface - base;
  const height = Math.min(900, Math.max(320, extent * 13));
  const scale = height / extent;
  const yOf = (level) => top + (surface - level) * scale;
  const total = height + top * 2 + 16;
  const svg = svgElement("svg", {
    class: "section",
    viewBox: `0 0 ${width} ${total}`,
    role: "img",
    "aria-label": `Section through ${model.modelKey}`,
  });
  const materialMap = new Map(materials.map((material) => [material.materialKey, material]));

  model.layers.forEach((layer, index) => {
    const layerTop = Number(layer.topLevel);
    const layerBase = index + 1 < model.layers.length ? Number(model.layers[index + 1].topLevel) : base;
    const topY = yOf(layerTop);
    const boxHeight = Math.max(1, yOf(layerBase) - topY);
    const materialHue = hue(layer.materialKey ?? "");
    svg.append(svgElement("rect", {
      class: "layer",
      x: columnLeft,
      y: topY.toFixed(1),
      width: columnWidth,
      height: boxHeight.toFixed(1),
      style: `--fill:hsl(${materialHue} 34% 76%);--fill-dark:hsl(${materialHue} 26% 30%)`,
    }));
    svg.append(svgElement("line", { class: "boundary", x1: columnLeft, y1: topY, x2: columnLeft + columnWidth, y2: topY }));
    svg.append(svgElement("text", { class: "level", x: columnLeft - 8, y: topY + 4 }, layerTop.toFixed(2)));
    svg.append(svgElement("text", { class: "depth", x: columnLeft - 8, y: topY + 17 }, `${(surface - layerTop).toFixed(2)} m`));
    const material = materialMap.get(layer.materialKey);
    const label = material?.name || layer.materialKey || "Unknown";
    if (boxHeight >= 20) {
      svg.append(svgElement("text", { class: "stratum", x: columnLeft + columnWidth / 2, y: topY + boxHeight / 2 + 4 }, label));
    } else {
      svg.append(svgElement("text", { class: "stratum-outside", x: columnLeft + columnWidth + 10, y: topY + boxHeight / 2 + 4 }, label));
    }
  });

  const baseY = yOf(base);
  svg.append(svgElement("line", { class: "boundary base", x1: columnLeft, y1: baseY, x2: columnLeft + columnWidth, y2: baseY }));
  svg.append(svgElement("text", { class: "level", x: columnLeft - 8, y: baseY + 4 }, base.toFixed(2)));
  svg.append(svgElement("text", { class: "depth", x: columnLeft - 8, y: baseY + 17 }, `${extent.toFixed(2)} m`));
  svg.append(svgElement("line", { class: "ground", x1: columnLeft - 14, y1: top, x2: columnLeft + columnWidth + 14, y2: top }));

  if (model.groundwater?.kind === "hydrostatic") {
    const depth = Number(model.groundwater.depth);
    const level = surface - depth;
    if (Number.isFinite(depth) && level <= surface && level >= base) {
      const y = yOf(level);
      const x = columnLeft + columnWidth;
      svg.append(svgElement("line", { class: "water", x1: columnLeft - 6, y1: y, x2: x + 6, y2: y }));
      svg.append(svgElement("polygon", { class: "water-mark", points: `${x + 12},${y - 6} ${x + 24},${y - 6} ${x + 18},${y + 4}` }));
      svg.append(svgElement("text", { class: "water-label", x: x + 30, y: y + 4 }, `${depth.toFixed(2)} m`));
    }
  }
  return svg;
}
