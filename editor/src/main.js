import "./style.css";
import { drawSection, hue } from "./section.js";
import {
  downloadGroundFile,
  downloadJson,
  normaliseDocument,
  openGroundFile,
} from "./document.js";
import { issueCounts, validateDocument } from "./validation.js";
import { sampleDocument } from "./sample.js";

const app = document.querySelector("#app");
const fileInput = document.querySelector("#file-input");

const state = {
  document: null,
  source: null,
  fileName: "ground-model.gm.json",
  active: "models",
  selectedModel: 0,
  selectedMaterial: 0,
  dirty: false,
  notice: null,
};

function element(tag, attributes = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2).toLowerCase(), value);
    else if (value !== undefined && value !== null) node.setAttribute(key, String(value));
  }
  for (const child of Array.isArray(children) ? children : [children]) {
    if (child !== undefined && child !== null) node.append(child);
  }
  return node;
}

function button(label, onClick, kind = "quiet", attributes = {}) {
  return element("button", { type: "button", class: `button ${kind}`, text: label, onclick: onClick, ...attributes });
}

function setNotice(message, kind = "info") {
  state.notice = { message, kind };
  render();
}

function changed(message) {
  state.dirty = true;
  state.notice = message ? { message, kind: "info" } : null;
}

function clone(value) {
  return structuredClone(value);
}

function loadDocument(result) {
  if (state.source?.database && state.source.database !== result.source?.database) state.source.database.close();
  state.document = normaliseDocument(result.document);
  state.source = result.source;
  state.fileName = result.fileName;
  state.active = "models";
  state.selectedModel = 0;
  state.selectedMaterial = 0;
  state.dirty = false;
  state.notice = result.source.kind === "gm"
    ? { kind: "info", message: "Loaded locally. Downloading writes an uncommitted working tree; use gm diff and gm commit afterwards." }
    : null;
  render();
}

async function openSelectedFile(file) {
  if (!file) return;
  try {
    app.classList.add("busy");
    loadDocument(await openGroundFile(file));
  } catch (error) {
    setNotice(error instanceof Error ? error.message : String(error), "error");
  } finally {
    app.classList.remove("busy");
    fileInput.value = "";
  }
}

fileInput.addEventListener("change", () => openSelectedFile(fileInput.files[0]));
window.addEventListener("beforeunload", (event) => {
  if (state.dirty) event.preventDefault();
});
window.addEventListener("dragover", (event) => event.preventDefault());
window.addEventListener("drop", (event) => {
  event.preventDefault();
  openSelectedFile(event.dataTransfer.files[0]);
});

function openPicker() {
  fileInput.click();
}

function newDocument() {
  loadDocument({
    document: {
      type: "gm.file/1",
      schemaVersion: "0.1.0",
      file: { name: "Untitled ground model" },
      materials: [],
      models: [],
    },
    source: { kind: "json" },
    fileName: "ground-model.gm.json",
  });
  state.dirty = true;
  render();
}

function useSample() {
  loadDocument({ document: clone(sampleDocument), source: { kind: "json" }, fileName: "example-route.gm.json" });
}

function renderLanding() {
  const page = element("main", { class: "landing" });
  page.append(
    element("div", { class: "mark", text: "gm" }),
    element("p", { class: "eyebrow", text: "GROUND MODEL TOOLS" }),
    element("h1", { text: "Read the ground. Shape the model." }),
    element("p", { class: "lede", text: "Open, inspect and edit 1D ground models in your browser. Your file stays on this device." }),
    element("div", { class: "landing-actions" }, [
      button("Open a ground file", openPicker, "primary"),
      button("New JSON file", newDocument),
      button("Try the example", useSample, "text"),
    ]),
    element("p", { class: "drop-note", text: "Drop a .gm or .gm.json file anywhere on this page" }),
    element("div", { class: "privacy-card" }, [
      element("span", { class: "privacy-icon", text: "⌁" }),
      element("div", {}, [
        element("strong", { text: "Local by design" }),
        element("p", { text: "The app has no server. SQLite and JSON files are read in browser memory and only leave when you download them." }),
      ]),
    ]),
  );
  if (state.notice) page.prepend(renderNotice());
  app.replaceChildren(page);
}

function renderNotice() {
  return element("div", { class: `notice ${state.notice.kind}` }, [
    element("span", { text: state.notice.message }),
    button("Dismiss", () => { state.notice = null; render(); }, "text compact"),
  ]);
}

function renderShell() {
  const issues = validateDocument(state.document);
  const counts = issueCounts(issues);
  const header = element("header", { class: "top" }, [
    element("div", { class: "brand" }, [
      element("span", { class: "brand-mark", text: "gm" }),
      element("button", {
        type: "button",
        class: "file-title",
        text: state.document.file.name,
        onclick: () => { state.active = "project"; render(); },
      }),
      element("span", { class: "format-badge", text: state.source.kind === "gm" ? "SQLite .gm" : "JSON" }),
      ...(state.dirty ? [element("span", { class: "dirty", title: "Not downloaded", text: "Edited" })] : []),
    ]),
    element("div", { class: "top-actions" }, [
      button("Open", openPicker),
      button("Export JSON", () => {
        downloadJson(state.document, state.fileName);
        setNotice("JSON export downloaded.", "success");
      }),
      button(state.source.kind === "gm" ? "Download .gm" : "Download JSON", savePrimary, "primary"),
    ]),
  ]);

  const tabs = [
    ["models", "Models", state.document.models.length],
    ["materials", "Materials", state.document.materials.length],
    ["validate", "Validation", counts.error + counts.warning],
    ["json", "JSON", null],
  ];
  const nav = element("nav", { class: "tabs", "aria-label": "Editor sections" }, tabs.map(([key, label, count]) =>
    element("button", {
      type: "button",
      class: state.active === key ? "on" : "",
      onclick: () => { state.active = key; render(); },
    }, [document.createTextNode(label), ...(count === null ? [] : [element("span", { class: "count", text: count })])]),
  ));

  const main = element("main", { class: "workspace" });
  if (state.notice) main.append(renderNotice());
  if (state.active === "models") main.append(renderModels(issues));
  else if (state.active === "materials") main.append(renderMaterials());
  else if (state.active === "validate") main.append(renderValidation(issues));
  else if (state.active === "project") main.append(renderProject());
  else main.append(renderJson());

  const footer = element("footer", {}, [
    element("span", { text: "Files stay in this browser" }),
    element("span", { text: state.source.kind === "gm" ? "History preserved · changes are not committed" : "JSON interchange document · no repository history" }),
  ]);
  app.replaceChildren(header, nav, main, footer);
}

function savePrimary() {
  try {
    if (state.source.kind === "gm") downloadGroundFile(state.document, state.source, state.fileName);
    else downloadJson(state.document, state.fileName);
    state.dirty = false;
    setNotice(state.source.kind === "gm"
      ? "Ground file downloaded. Review it with gm diff, then commit it with the CLI."
      : "JSON file downloaded.", "success");
  } catch (error) {
    setNotice(error instanceof Error ? error.message : String(error), "error");
  }
}

function field(label, value, onChange, options = {}) {
  const input = element(options.multiline ? "textarea" : "input", {
    type: options.type ?? "text",
    value: value ?? "",
    placeholder: options.placeholder,
    step: options.step,
    readonly: options.readOnly ? "readonly" : null,
    onchange: (event) => {
      let next = event.target.value;
      if (options.type === "number") next = next === "" ? undefined : Number(next);
      onChange(next);
      changed();
      render();
    },
  });
  if (options.multiline) input.value = value ?? "";
  return element("label", { class: options.wide ? "field wide" : "field" }, [
    element("span", { text: label }),
    input,
    ...(options.help ? [element("small", { text: options.help })] : []),
  ]);
}

function setOptional(object, key, value) {
  if (value === "" || value === undefined) delete object[key];
  else object[key] = value;
}

function renderProject() {
  const file = state.document.file;
  return element("section", { class: "narrow-page" }, [
    element("div", { class: "page-heading" }, [
      element("div", {}, [element("p", { class: "eyebrow", text: "FILE METADATA" }), element("h1", { text: "Project" })]),
    ]),
    element("div", { class: "panel form-grid" }, [
      field("Name", file.name, (value) => { file.name = value; }, { wide: true }),
      field("Description", file.description, (value) => setOptional(file, "description", value), { multiline: true, wide: true }),
      field("Horizontal CRS", file.crs, (value) => setOptional(file, "crs", value), { placeholder: "EPSG:27700" }),
      field("Vertical datum", file.verticalDatum, (value) => setOptional(file, "verticalDatum", value), { placeholder: "Ordnance Datum Newlyn" }),
      field("Schema version", state.document.schemaVersion, () => {}, { readOnly: true, help: "Managed by gm; shown for reference." }),
      ...(state.document.sourceCommit ? [field("Source commit", state.document.sourceCommit, () => {}, { readOnly: true, help: "The revision this working tree came from." })] : []),
    ]),
  ]);
}

function renderModels(issues) {
  const models = state.document.models;
  if (state.selectedModel >= models.length) state.selectedModel = Math.max(0, models.length - 1);
  const sidebar = element("aside", { class: "sidebar" }, [
    element("div", { class: "sidebar-heading" }, [
      element("span", { text: `${models.length} model${models.length === 1 ? "" : "s"}` }),
      button("+ Add", addModel, "text compact"),
    ]),
    element("div", { class: "side-list" }, models.map((model, index) => {
      const modelIssues = issues.filter((item) => item.modelKey === model.modelKey);
      return element("button", {
        type: "button",
        class: index === state.selectedModel ? "side-item on" : "side-item",
        onclick: () => { state.selectedModel = index; render(); },
      }, [
        element("span", { class: "side-title", text: model.modelKey || "Untitled model" }),
        element("span", { class: "side-subtitle", text: model.name || `${model.layers.length} layers` }),
        ...(modelIssues.length ? [element("span", { class: `issue-dot ${modelIssues.some((item) => item.severity === "error") ? "error" : "warning"}` })] : []),
      ]);
    })),
  ]);
  const content = models.length ? renderModelEditor(models[state.selectedModel], issues) : renderEmpty("No models yet", "Add a vertical succession to start drawing the ground.", addModel, "Add the first model");
  return element("div", { class: "split-layout" }, [sidebar, element("section", { class: "editor-pane" }, [content])]);
}

function addModel() {
  const materialKey = state.document.materials[0]?.materialKey ?? "";
  state.document.models.push({
    modelKey: `MODEL-${String(state.document.models.length + 1).padStart(3, "0")}`,
    modelType: "1d",
    gammaW: 9.81,
    groundwater: { kind: "unknown" },
    settings: { allowGaps: false },
    layers: materialKey ? [{ topLevel: 0, materialKey }] : [],
  });
  state.selectedModel = state.document.models.length - 1;
  changed();
  render();
}

function renderModelEditor(model, allIssues) {
  const modelIssues = allIssues.filter((item) => item.modelKey === model.modelKey);
  const heading = element("div", { class: "page-heading" }, [
    element("div", {}, [element("p", { class: "eyebrow", text: "1D GROUND MODEL" }), element("h1", { text: model.modelKey || "Untitled model" }), element("p", { class: "subtitle", text: model.name || "Unnamed model" })]),
    element("div", { class: "heading-actions" }, [
      button("Duplicate", () => {
        const copy = clone(model);
        copy.modelKey = `${model.modelKey}-COPY`;
        state.document.models.splice(state.selectedModel + 1, 0, copy);
        state.selectedModel += 1;
        changed(); render();
      }),
      button("Delete", () => {
        if (confirm(`Delete model '${model.modelKey}'?`)) {
          state.document.models.splice(state.selectedModel, 1); changed(); render();
        }
      }, "danger"),
    ]),
  ]);
  const form = element("div", { class: "panel form-grid" }, [
    field("Model key", model.modelKey, (value) => { model.modelKey = value; }),
    field("Name", model.name, (value) => setOptional(model, "name", value)),
    field("Description", model.description, (value) => setOptional(model, "description", value), { multiline: true, wide: true }),
    field("Surface level", model.surfaceLevel, (value) => setOptional(model, "surfaceLevel", value), { type: "number", step: "any" }),
    field("Base level", model.baseLevel, (value) => setOptional(model, "baseLevel", value), { type: "number", step: "any" }),
    field("Easting / X", model.x, (value) => setOptional(model, "x", value), { type: "number", step: "any" }),
    field("Northing / Y", model.y, (value) => setOptional(model, "y", value), { type: "number", step: "any" }),
  ]);

  const groundwaterKind = model.groundwater?.kind ?? "unknown";
  const groundwaterSelect = element("select", { onchange: (event) => {
    const kind = event.target.value;
    model.groundwater = kind === "hydrostatic" ? { kind, depth: 0 } : { kind };
    changed(); render();
  }}, ["unknown", "dry", "hydrostatic"].map((kind) => element("option", { value: kind, text: kind, ...(kind === groundwaterKind ? { selected: "selected" } : {}) })));
  const waterPanel = element("div", { class: "panel inline-form" }, [
    element("label", { class: "field" }, [element("span", { text: "Groundwater" }), groundwaterSelect]),
    ...(groundwaterKind === "hydrostatic" ? [field("Depth below ground", model.groundwater.depth, (value) => { model.groundwater.depth = value ?? 0; }, { type: "number", step: "any" })] : []),
    field("Water unit weight", model.gammaW ?? 9.81, (value) => { model.gammaW = value ?? 9.81; }, { type: "number", step: "any" }),
  ]);

  const layers = renderLayers(model);
  const drawing = element("div", { class: "panel section-panel" }, [
    element("div", { class: "panel-title" }, [element("h2", { text: "Section" }), element("span", { text: `${model.layers.length} strata` })]),
    drawSection(model, state.document.materials),
  ]);
  const issuePanel = modelIssues.length ? element("div", { class: "inline-issues" }, modelIssues.map(renderIssue)) : null;
  return element("div", {}, [heading, ...(issuePanel ? [issuePanel] : []), element("div", { class: "model-grid" }, [element("div", { class: "model-fields" }, [form, waterPanel, layers]), drawing])]);
}

function renderLayers(model) {
  const materials = state.document.materials;
  const rows = model.layers.map((layer, index) => {
    const materialSelect = element("select", { onchange: (event) => { layer.materialKey = event.target.value; changed(); render(); } }, materials.map((material) =>
      element("option", { value: material.materialKey, text: material.name ? `${material.materialKey} — ${material.name}` : material.materialKey, ...(material.materialKey === layer.materialKey ? { selected: "selected" } : {}) })
    ));
    if (!materials.some((material) => material.materialKey === layer.materialKey)) {
      materialSelect.prepend(element("option", { value: layer.materialKey, text: `${layer.materialKey || "Choose material"} (missing)`, selected: "selected" }));
    }
    return element("div", { class: "layer-row" }, [
      element("span", { class: "grip", text: String(index + 1) }),
      element("label", { class: "mini-field level-input" }, [element("span", { text: "Top level" }), element("input", {
        type: "number", step: "any", value: layer.topLevel,
        onchange: (event) => { layer.topLevel = Number(event.target.value); changed(); render(); },
      })]),
      element("label", { class: "mini-field material-input" }, [element("span", { text: "Material" }), materialSelect]),
      element("label", { class: "mini-field description-input" }, [element("span", { text: "Description" }), element("input", {
        value: layer.description ?? "", placeholder: "Optional",
        onchange: (event) => { setOptional(layer, "description", event.target.value); changed(); render(); },
      })]),
      element("div", { class: "row-actions" }, [
        button("↑", () => moveLayer(model, index, -1), "icon", { title: "Move up", disabled: index === 0 ? "disabled" : null }),
        button("↓", () => moveLayer(model, index, 1), "icon", { title: "Move down", disabled: index === model.layers.length - 1 ? "disabled" : null }),
        button("×", () => { model.layers.splice(index, 1); changed(); render(); }, "icon danger", { title: "Remove layer" }),
      ]),
    ]);
  });
  return element("div", { class: "panel layers-panel" }, [
    element("div", { class: "panel-title" }, [element("h2", { text: "Layers" }), button("+ Add layer", () => {
      const previous = model.layers.at(-1);
      model.layers.push({ topLevel: previous ? Number(previous.topLevel) - 1 : Number(model.surfaceLevel ?? 0), materialKey: materials[0]?.materialKey ?? "" });
      changed(); render();
    }, "text compact")]),
    ...(rows.length ? rows : [element("p", { class: "empty", text: "No layers yet. Add a material first if the list is empty." })]),
  ]);
}

function moveLayer(model, index, offset) {
  const target = index + offset;
  if (target < 0 || target >= model.layers.length) return;
  [model.layers[index], model.layers[target]] = [model.layers[target], model.layers[index]];
  changed(); render();
}

function renderMaterials() {
  const materials = state.document.materials;
  if (state.selectedMaterial >= materials.length) state.selectedMaterial = Math.max(0, materials.length - 1);
  const sidebar = element("aside", { class: "sidebar" }, [
    element("div", { class: "sidebar-heading" }, [element("span", { text: `${materials.length} material${materials.length === 1 ? "" : "s"}` }), button("+ Add", addMaterial, "text compact")]),
    element("div", { class: "side-list" }, materials.map((material, index) => element("button", {
      type: "button", class: index === state.selectedMaterial ? "side-item on" : "side-item",
      onclick: () => { state.selectedMaterial = index; render(); },
    }, [element("span", { class: "material-swatch", style: `--h:${hue(material.materialKey)}` }), element("span", { class: "side-title", text: material.materialKey || "Untitled material" }), element("span", { class: "side-subtitle", text: material.name || material.soilClass || "Unnamed material" })]))),
  ]);
  const content = materials.length ? renderMaterialEditor(materials[state.selectedMaterial]) : renderEmpty("No materials yet", "Materials are shared by layers across every model.", addMaterial, "Add the first material");
  return element("div", { class: "split-layout" }, [sidebar, element("section", { class: "editor-pane" }, [content])]);
}

function addMaterial() {
  state.document.materials.push({ materialKey: `MATERIAL_${state.document.materials.length + 1}`, properties: {}, constitutiveModels: [] });
  state.selectedMaterial = state.document.materials.length - 1;
  changed(); render();
}

function renderMaterialEditor(material) {
  const originalKey = material.materialKey;
  return element("div", {}, [
    element("div", { class: "page-heading" }, [
      element("div", {}, [element("p", { class: "eyebrow", text: "SHARED MATERIAL" }), element("h1", { text: material.materialKey || "Untitled material" }), element("p", { class: "subtitle", text: material.name || "Unnamed material" })]),
      button("Delete", () => {
        const uses = state.document.models.reduce((count, model) => count + model.layers.filter((layer) => layer.materialKey === material.materialKey).length, 0);
        if (uses) return setNotice(`Cannot delete '${material.materialKey}': ${uses} layer${uses === 1 ? "" : "s"} still use it.`, "error");
        if (confirm(`Delete material '${material.materialKey}'?`)) { state.document.materials.splice(state.selectedMaterial, 1); changed(); render(); }
      }, "danger"),
    ]),
    element("div", { class: "panel form-grid" }, [
      field("Material key", material.materialKey, (value) => {
        material.materialKey = value;
        state.document.models.forEach((model) => model.layers.forEach((layer) => { if (layer.materialKey === originalKey) layer.materialKey = value; }));
      }),
      field("Name", material.name, (value) => setOptional(material, "name", value)),
      field("Description", material.description, (value) => setOptional(material, "description", value), { multiline: true, wide: true }),
      field("Soil class", material.soilClass, (value) => setOptional(material, "soilClass", value)),
    ]),
    renderProperties(material),
    renderAdvancedJson(material),
  ]);
}

function renderProperties(material) {
  const entries = Object.entries(material.properties ?? {});
  return element("div", { class: "panel properties-panel" }, [
    element("div", { class: "panel-title" }, [element("h2", { text: "Properties" }), button("+ Add property", () => {
      material.properties ??= {};
      let key = "property";
      let suffix = 1;
      while (key in material.properties) key = `property${++suffix}`;
      material.properties[key] = { unit: "" };
      changed(); render();
    }, "text compact")]),
    ...(entries.length ? [element("div", { class: "property-table" }, entries.map(([key, bounded]) => {
      const row = element("div", { class: "property-row" });
      const values = [key, bounded.value, bounded.lower, bounded.upper, bounded.unit];
      const labels = ["Property", "Value", "Lower", "Upper", "Unit"];
      values.forEach((value, index) => row.append(element("label", { class: `mini-field property-${index}` }, [
        element("span", { text: labels[index] }),
        element("input", { value: value ?? "", type: index > 0 && index < 4 ? "number" : "text", step: "any", onchange: (event) => {
          if (index === 0) {
            const nextKey = event.target.value;
            delete material.properties[key];
            material.properties[nextKey] = bounded;
          } else {
            const prop = [null, "value", "lower", "upper", "unit"][index];
            const next = event.target.value;
            if (next === "") delete bounded[prop];
            else bounded[prop] = index < 4 ? Number(next) : next;
          }
          changed(); render();
        }}),
      ])));
      row.append(button("×", () => { delete material.properties[key]; changed(); render(); }, "icon danger", { title: "Remove property" }));
      return row;
    }))] : [element("p", { class: "empty", text: "No material properties yet." })]),
  ]);
}

function renderAdvancedJson(material) {
  const details = element("details", { class: "panel advanced" });
  details.append(element("summary", { text: "Advanced constitutive models (JSON)" }));
  const textarea = element("textarea", { class: "code-editor", spellcheck: "false" });
  textarea.value = JSON.stringify(material.constitutiveModels ?? [], null, 2);
  details.append(element("p", { class: "note", text: "Profiles, drainage and open constitutive-model parameters are preserved here without narrowing the format." }), textarea,
    button("Apply advanced JSON", () => {
      try {
        const value = JSON.parse(textarea.value);
        if (!Array.isArray(value)) throw new Error("Constitutive models must be a JSON array.");
        material.constitutiveModels = value;
        changed("Advanced material data applied."); render();
      } catch (error) { setNotice(error.message, "error"); }
    }));
  return details;
}

function renderValidation(issues) {
  const counts = issueCounts(issues);
  const summaryClass = counts.error ? "validation-summary has-errors" : "validation-summary clean";
  return element("section", { class: "narrow-page" }, [
    element("div", { class: "page-heading" }, [element("div", {}, [element("p", { class: "eyebrow", text: "LIVE CHECKS" }), element("h1", { text: "Validation" }), element("p", { class: "subtitle", text: "The same core coherence checks used before a gm commit." })])]),
    element("div", { class: summaryClass }, [
      element("div", { class: "score", text: counts.error ? "!" : "✓" }),
      element("div", {}, [element("strong", { text: counts.error ? `${counts.error} error${counts.error === 1 ? "" : "s"} must be fixed` : "No blocking errors" }), element("p", { text: `${counts.warning} warning${counts.warning === 1 ? "" : "s"} · ${state.document.models.length} models · ${state.document.materials.length} materials` })]),
    ]),
    ...(issues.length ? [element("div", { class: "issue-list panel" }, issues.map(renderIssue))] : [element("div", { class: "panel empty-state small" }, [element("h2", { text: "Everything looks coherent" }), element("p", { text: "No validation errors or warnings were found." })])]),
  ]);
}

function renderIssue(item) {
  const click = item.modelKey ? () => {
    const index = state.document.models.findIndex((model) => model.modelKey === item.modelKey);
    if (index >= 0) { state.selectedModel = index; state.active = "models"; render(); }
  } : null;
  return element(click ? "button" : "div", { ...(click ? { type: "button", onclick: click } : {}), class: `issue ${item.severity}` }, [
    element("span", { class: "severity", text: item.severity }),
    element("div", {}, [element("strong", { text: item.modelKey || "File" }), element("p", { text: item.message }), element("code", { text: item.fieldPath })]),
  ]);
}

function renderJson() {
  const textarea = element("textarea", { class: "json-editor", spellcheck: "false", "aria-label": "Ground model JSON" });
  textarea.value = JSON.stringify(state.document, null, 2);
  return element("section", { class: "json-page" }, [
    element("div", { class: "page-heading" }, [
      element("div", {}, [element("p", { class: "eyebrow", text: "INTERCHANGE DOCUMENT" }), element("h1", { text: "JSON" }), element("p", { class: "subtitle", text: "Full-fidelity access to fields that do not yet have a visual control." })]),
      button("Apply JSON", () => {
        try {
          state.document = normaliseDocument(JSON.parse(textarea.value));
          changed("JSON applied to the editor."); render();
        } catch (error) { setNotice(error.message, "error"); }
      }, "primary"),
    ]),
    textarea,
  ]);
}

function renderEmpty(title, description, action, actionLabel) {
  return element("div", { class: "panel empty-state" }, [element("div", { class: "empty-symbol", text: "↧" }), element("h1", { text: title }), element("p", { text: description }), button(actionLabel, action, "primary")]);
}

function render() {
  if (!state.document) renderLanding();
  else renderShell();
}

render();
