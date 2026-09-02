import { describe, expect, it } from "vitest";
import {
  KNOWN_CONSTITUTIVE_KINDS,
  constitutiveKinds,
  defaultUnit,
  materialProperties,
  nextTerm,
  unitsFor,
} from "./vocabulary.js";
import { validateDocument } from "./validation.js";

const material = (extra) => ({
  type: "gm.file/1",
  file: { name: "V", crs: "EPSG:27700", verticalDatum: "ODN" },
  materials: [{ materialKey: "M", properties: {}, constitutiveModels: [], ...extra }],
  models: [],
});

describe("the suggestion vocabulary", () => {
  it("offers a unit for every term it suggests", () => {
    const terms = [...materialProperties, ...constitutiveKinds.flatMap((kind) => kind.parameters)];
    for (const term of terms) {
      expect(term.units.length, `${term.key} suggests no unit`).toBeGreaterThan(0);
      expect(term.label).toBeTruthy();
    }
  });

  // The whole point of choosing a kind from a list: what the editor offers and
  // what the validator recognises are the same set, read from the same file.
  it("offers only kinds the validator recognises", () => {
    for (const { kind } of constitutiveKinds) {
      const issues = validateDocument(material({ constitutiveModels: [{ id: "cm-1", kind, parameters: {} }] }));
      expect(issues.filter((item) => item.fieldPath.endsWith(".kind"))).toEqual([]);
    }
    expect(KNOWN_CONSTITUTIVE_KINDS.has("something-newer")).toBe(false);
  });

  it("suggests the unit a term is normally given in", () => {
    expect(defaultUnit("unitWeight")).toBe("kN/m3");
    expect(defaultUnit("permeability")).toBe("m/s");
    expect(defaultUnit("frictionAngleDeg", "mohr-coulomb")).toBe("deg");
    expect(unitsFor("frictionAngleDeg", "mohr-coulomb")).toEqual(["deg"]);
  });

  // A property the vocabulary has never heard of still needs a unit, so it gets
  // every unit rather than none.
  it("falls back to the full unit list for an unknown term", () => {
    expect(unitsFor("someHouseParameter")).toContain("kPa");
    expect(defaultUnit("someHouseParameter")).toBeUndefined();
  });

  it("adds the first term a material is missing", () => {
    expect(nextTerm([]).key).toBe("unitWeight");
    expect(nextTerm(["unitWeight"]).key).toBe("saturatedUnitWeight");
    expect(nextTerm([], "undrained-tresca").key).toBe("undrainedShearStrength");
    expect(nextTerm(constitutiveKinds[0].parameters.map((p) => p.key), "mohr-coulomb")).toBeUndefined();
  });
});
