import { createRequire } from "node:module";
import initSqlJs from "sql.js";
import { beforeAll, describe, expect, it } from "vitest";
import { sqliteToDocument, writeWorkingTree } from "./document.js";

const require = createRequire(import.meta.url);
let SQL;

beforeAll(async () => {
  SQL = await initSqlJs({ locateFile: () => require.resolve("sql.js/dist/sql-wasm.wasm") });
});

function groundFile() {
  const database = new SQL.Database();
  database.exec(`
    PRAGMA application_id = 1196246092;
    CREATE TABLE gm_config (id INTEGER PRIMARY KEY, file_id TEXT, schema_version TEXT);
    CREATE TABLE gm_ref (name TEXT PRIMARY KEY, commit_hash TEXT);
    CREATE TABLE file_metadata (
      id INTEGER PRIMARY KEY, file_id TEXT, name TEXT, description TEXT,
      schema_version TEXT, created_at TEXT, updated_at TEXT, crs TEXT,
      vertical_datum TEXT, metadata TEXT
    );
    CREATE TABLE materials (
      id TEXT PRIMARY KEY, file_id TEXT, material_key TEXT, name TEXT,
      description TEXT, soil_class TEXT, properties TEXT, constitutive_models TEXT,
      provenance TEXT, metadata TEXT, created_at TEXT, updated_at TEXT
    );
    CREATE TABLE ground_models (
      id TEXT PRIMARY KEY, file_id TEXT, model_key TEXT, name TEXT, description TEXT,
      model_type TEXT, surface_level REAL, base_level REAL, x REAL, y REAL,
      gamma_w REAL, groundwater TEXT, settings TEXT, metadata TEXT,
      created_at TEXT, updated_at TEXT
    );
    CREATE TABLE ground_layers (
      id TEXT PRIMARY KEY, ground_model_id TEXT, layer_order INTEGER,
      top_level REAL, base_level REAL, material_id TEXT, material_key TEXT,
      description TEXT, source TEXT, generated_from_profile INTEGER, metadata TEXT
    );
    CREATE TABLE model_issues (id TEXT PRIMARY KEY);
    INSERT INTO gm_config VALUES (1, 'file-1', '0.1.0');
    INSERT INTO gm_ref VALUES ('HEAD', 'sha256-abc');
    INSERT INTO file_metadata VALUES
      (1, 'file-1', 'Route', NULL, '0.1.0', '2026-01-01Z', '2026-01-01Z',
       'EPSG:27700', 'ODN', NULL);
    INSERT INTO materials VALUES
      ('old-material-id', 'file-1', 'CLAY', 'Clay', NULL, 'clay',
       '{"unitWeight":{"value":20,"unit":"kN/m3"}}', '[]', NULL, NULL,
       '2026-01-01Z', '2026-01-01Z');
    INSERT INTO ground_models VALUES
      ('old-model-id', 'file-1', 'BH-01', 'Borehole 1', NULL, '1d', 10, 0,
       100, 200, 9.81, '{"kind":"hydrostatic","depth":2}',
       '{"allowGaps":false}', NULL, '2026-01-01Z', '2026-01-01Z');
    INSERT INTO ground_layers VALUES
      ('old-model-id:001', 'old-model-id', 1, 10, 0, 'old-material-id',
       'CLAY', NULL, NULL, 0, NULL);
  `);
  return database;
}

function scalar(database, sql) {
  return database.exec(sql)[0].values[0][0];
}

describe("SQLite .gm working trees", () => {
  it("reads a repository and preserves its source revision", () => {
    const database = groundFile();
    const document = sqliteToDocument(database);
    expect(document.file.name).toBe("Route");
    expect(document.sourceCommit).toBe("sha256-abc");
    expect(document.models[0].layers[0]).toMatchObject({ topLevel: 10, materialKey: "CLAY" });
    database.close();
  });

  it("rewrites only the materialised working tree", () => {
    const database = groundFile();
    const document = sqliteToDocument(database);
    document.models[0].layers[0].topLevel = 9.5;
    document.materials[0].properties.unitWeight.value = 20.5;
    writeWorkingTree(database, document);

    expect(scalar(database, "SELECT top_level FROM ground_layers")).toBe(9.5);
    expect(JSON.parse(scalar(database, "SELECT properties FROM materials")).unitWeight.value).toBe(20.5);
    expect(scalar(database, "SELECT commit_hash FROM gm_ref WHERE name = 'HEAD'")).toBe("sha256-abc");

    const reopened = new SQL.Database(database.export());
    expect(sqliteToDocument(reopened).models[0].layers[0].topLevel).toBe(9.5);
    reopened.close();
    database.close();
  });
});
