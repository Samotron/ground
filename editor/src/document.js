import initSqlJs from "sql.js";
import wasmUrl from "sql.js/dist/sql-wasm.wasm?url";

const GM_APPLICATION_ID = 0x474d444c;
let sqlPromise;

function sql() {
  sqlPromise ??= initSqlJs({ locateFile: () => wasmUrl });
  return sqlPromise;
}

function rows(database, statement, parameters = []) {
  const prepared = database.prepare(statement);
  try {
    prepared.bind(parameters);
    const result = [];
    while (prepared.step()) result.push(prepared.getAsObject());
    return result;
  } finally {
    prepared.free();
  }
}

function one(database, statement, parameters = []) {
  return rows(database, statement, parameters)[0];
}

function parseJson(value, fallback) {
  if (value === null || value === undefined || value === "") return fallback;
  return JSON.parse(value);
}

function optionalJson(value) {
  return value === undefined || value === null ? null : JSON.stringify(value);
}

export function normaliseDocument(input) {
  if (!input || typeof input !== "object") throw new Error("The file does not contain a JSON object.");
  if (input.type !== "gm.file/1") throw new Error(`Expected a 'gm.file/1' document, found '${input.type ?? "no type"}'.`);
  return {
    ...input,
    schemaVersion: input.schemaVersion ?? "0.1.0",
    file: { name: "Untitled ground model", ...(input.file ?? {}) },
    materials: Array.isArray(input.materials) ? input.materials : [],
    models: Array.isArray(input.models) ? input.models : [],
  };
}

export function sqliteToDocument(database) {
  const applicationId = Number(one(database, "PRAGMA application_id")?.application_id);
  if (applicationId !== GM_APPLICATION_ID) throw new Error("This SQLite file is not a gm ground-model repository.");
  const config = one(database, "SELECT file_id, schema_version FROM gm_config WHERE id = 1");
  const file = one(database, `
    SELECT name, description, crs, vertical_datum, metadata
    FROM file_metadata WHERE id = 1
  `);
  if (!config || !file) throw new Error("The gm file has no readable working tree.");

  const materials = rows(database, `
    SELECT material_key, name, description, soil_class, properties,
           constitutive_models, provenance, metadata
    FROM materials ORDER BY material_key
  `).map((material) => ({
    materialKey: material.material_key,
    ...(material.name === null ? {} : { name: material.name }),
    ...(material.description === null ? {} : { description: material.description }),
    ...(material.soil_class === null ? {} : { soilClass: material.soil_class }),
    properties: parseJson(material.properties, {}),
    constitutiveModels: parseJson(material.constitutive_models, []),
    ...(material.provenance === null ? {} : { provenance: parseJson(material.provenance, null) }),
    ...(material.metadata === null ? {} : { metadata: parseJson(material.metadata, null) }),
  }));

  const models = rows(database, `
    SELECT id, model_key, name, description, model_type, surface_level,
           base_level, x, y, gamma_w, groundwater, settings, metadata
    FROM ground_models ORDER BY model_key
  `).map((model) => {
    const layers = rows(database, `
      SELECT top_level, material_key, description, source,
             generated_from_profile, metadata
      FROM ground_layers WHERE ground_model_id = ? ORDER BY layer_order
    `, [model.id]).map((layer) => ({
      topLevel: layer.top_level,
      materialKey: layer.material_key,
      ...(layer.description === null ? {} : { description: layer.description }),
      ...(layer.source === null ? {} : { source: parseJson(layer.source, null) }),
      ...(layer.generated_from_profile ? { generatedFromProfile: true } : {}),
      ...(layer.metadata === null ? {} : { metadata: parseJson(layer.metadata, null) }),
    }));
    return {
      modelKey: model.model_key,
      ...(model.name === null ? {} : { name: model.name }),
      ...(model.description === null ? {} : { description: model.description }),
      modelType: model.model_type,
      ...(model.surface_level === null ? {} : { surfaceLevel: model.surface_level }),
      ...(model.base_level === null ? {} : { baseLevel: model.base_level }),
      ...(model.x === null ? {} : { x: model.x }),
      ...(model.y === null ? {} : { y: model.y }),
      gammaW: model.gamma_w,
      groundwater: parseJson(model.groundwater, { kind: "unknown" }),
      settings: parseJson(model.settings, { allowGaps: false }),
      ...(model.metadata === null ? {} : { metadata: parseJson(model.metadata, null) }),
      layers,
    };
  });

  const sourceCommit = one(database, "SELECT commit_hash FROM gm_ref WHERE name = 'HEAD'")?.commit_hash;
  return normaliseDocument({
    type: "gm.file/1",
    schemaVersion: config.schema_version,
    ...(sourceCommit ? { sourceCommit } : {}),
    file: {
      name: file.name,
      ...(file.description === null ? {} : { description: file.description }),
      ...(file.crs === null ? {} : { crs: file.crs }),
      ...(file.vertical_datum === null ? {} : { verticalDatum: file.vertical_datum }),
      ...(file.metadata === null ? {} : { metadata: parseJson(file.metadata, null) }),
    },
    materials,
    models,
  });
}

export async function openGroundFile(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const sqliteHeader = new TextDecoder().decode(bytes.slice(0, 16)) === "SQLite format 3\0";
  if (sqliteHeader || file.name.toLowerCase().endsWith(".gm")) {
    const SQL = await sql();
    const database = new SQL.Database(bytes);
    try {
      return { document: sqliteToDocument(database), source: { kind: "gm", database }, fileName: file.name };
    } catch (error) {
      database.close();
      throw error;
    }
  }
  const text = new TextDecoder().decode(bytes);
  return { document: normaliseDocument(JSON.parse(text)), source: { kind: "json" }, fileName: file.name };
}

function currentRowsByKey(database, table, key) {
  return new Map(rows(database, `SELECT ${key}, created_at, updated_at FROM ${table}`).map((row) => [row[key], row]));
}

export function writeWorkingTree(database, document) {
  const fileRow = one(database, "SELECT file_id, schema_version, created_at, updated_at FROM file_metadata WHERE id = 1");
  if (!fileRow) throw new Error("The gm file has no file_metadata row.");
  const materialHistory = currentRowsByKey(database, "materials", "material_key");
  const modelHistory = currentRowsByKey(database, "ground_models", "model_key");
  const now = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

  database.run("PRAGMA foreign_keys = OFF");
  database.run("BEGIN IMMEDIATE");
  try {
    database.run(`
      UPDATE file_metadata
      SET name = ?, description = ?, crs = ?, vertical_datum = ?, metadata = ?
      WHERE id = 1
    `, [document.file.name, document.file.description ?? null, document.file.crs ?? null,
      document.file.verticalDatum ?? null, optionalJson(document.file.metadata)]);
    database.run("DELETE FROM model_issues");
    database.run("DELETE FROM ground_layers");
    database.run("DELETE FROM ground_models");
    database.run("DELETE FROM materials");

    const materialIds = new Map();
    document.materials.forEach((material, index) => {
      const id = `editor-material-${index + 1}`;
      materialIds.set(material.materialKey, id);
      const history = materialHistory.get(material.materialKey);
      database.run(`
        INSERT INTO materials
          (id, file_id, material_key, name, description, soil_class, properties,
           constitutive_models, provenance, metadata, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `, [id, fileRow.file_id, material.materialKey, material.name ?? null,
        material.description ?? null, material.soilClass ?? null,
        JSON.stringify(material.properties ?? {}), JSON.stringify(material.constitutiveModels ?? []),
        optionalJson(material.provenance), optionalJson(material.metadata),
        history?.created_at ?? now, history?.updated_at ?? now]);
    });

    document.models.forEach((model, modelIndex) => {
      const id = `editor-model-${modelIndex + 1}`;
      const history = modelHistory.get(model.modelKey);
      database.run(`
        INSERT INTO ground_models
          (id, file_id, model_key, name, description, model_type, surface_level,
           base_level, x, y, gamma_w, groundwater, settings, metadata, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      `, [id, fileRow.file_id, model.modelKey, model.name ?? null, model.description ?? null,
        model.modelType ?? "1d", model.surfaceLevel ?? null, model.baseLevel ?? null,
        model.x ?? null, model.y ?? null, model.gammaW ?? 9.81,
        JSON.stringify(model.groundwater ?? { kind: "unknown" }),
        JSON.stringify(model.settings ?? { allowGaps: false }), optionalJson(model.metadata),
        history?.created_at ?? now, history?.updated_at ?? now]);

      model.layers.forEach((layer, layerIndex) => {
        const materialId = materialIds.get(layer.materialKey);
        if (!materialId) throw new Error(`Model '${model.modelKey}' references unknown material '${layer.materialKey}'.`);
        const baseLevel = layerIndex + 1 < model.layers.length
          ? model.layers[layerIndex + 1].topLevel
          : model.baseLevel ?? null;
        database.run(`
          INSERT INTO ground_layers
            (id, ground_model_id, layer_order, top_level, base_level, material_id,
             material_key, description, source, generated_from_profile, metadata)
          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        `, [`${id}:${String(layerIndex + 1).padStart(3, "0")}`, id, layerIndex + 1,
          layer.topLevel, baseLevel, materialId, layer.materialKey, layer.description ?? null,
          optionalJson(layer.source), layer.generatedFromProfile ? 1 : 0, optionalJson(layer.metadata)]);
      });
    });
    database.run("COMMIT");
  } catch (error) {
    database.run("ROLLBACK");
    throw error;
  } finally {
    database.run("PRAGMA foreign_keys = ON");
  }
}

export function sortedDocument(document) {
  const copy = structuredClone(document);
  copy.materials.sort((a, b) => a.materialKey.localeCompare(b.materialKey));
  copy.models.sort((a, b) => a.modelKey.localeCompare(b.modelKey));
  return copy;
}

export function downloadJson(document, fileName = "ground-model.gm.json") {
  const text = `${JSON.stringify(sortedDocument(document), null, 2)}\n`;
  triggerDownload(new Blob([text], { type: "application/json" }), jsonFileName(fileName));
}

export function downloadGroundFile(document, source, fileName) {
  if (source.kind !== "gm" || !source.database) throw new Error("No SQLite ground file is open.");
  writeWorkingTree(source.database, document);
  triggerDownload(new Blob([source.database.export()], { type: "application/vnd.sqlite3" }), gmFileName(fileName));
}

function jsonFileName(fileName) {
  if (fileName.toLowerCase().endsWith(".gm.json")) return fileName;
  if (fileName.toLowerCase().endsWith(".gm")) return `${fileName}.json`;
  return fileName.toLowerCase().endsWith(".json") ? fileName : `${fileName}.gm.json`;
}

function gmFileName(fileName) {
  return fileName.toLowerCase().endsWith(".gm") ? fileName : `${fileName}.gm`;
}

function triggerDownload(blob, fileName) {
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = fileName;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
