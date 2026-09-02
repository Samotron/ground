// The suggestion lists behind every dropdown in the material editor, and the
// set of constitutive kinds the validator recognises.
//
// `assets/vocabulary.json` is read by gm_core as well, so the editor cannot
// offer a constitutive kind that `gm validate` would then warn about. It is a
// suggestion list, not a schema: the format keeps property names, units, soil
// classes and kinds open, and anything typed outside these lists round-trips
// untouched.
import vocabulary from "../../assets/vocabulary.json";

export const soilClasses = vocabulary.soilClasses;
export const materialProperties = vocabulary.properties;
export const constitutiveKinds = vocabulary.constitutiveKinds;

export const KNOWN_CONSTITUTIVE_KINDS = new Set(constitutiveKinds.map((kind) => kind.kind));

// Every unit anyone suggests anywhere, for a property this file has never heard
// of: better a long list than none.
export const allUnits = [...new Set([
  ...materialProperties.flatMap((property) => property.units),
  ...constitutiveKinds.flatMap((kind) => kind.parameters.map((parameter) => parameter.units).flat()),
])];

const propertyIndex = new Map(materialProperties.map((property) => [property.key, property]));
const kindIndex = new Map(constitutiveKinds.map((kind) => [kind.kind, kind]));

export function constitutiveKind(kind) {
  return kindIndex.get(kind);
}

export function kindParameters(kind) {
  return kindIndex.get(kind)?.parameters ?? [];
}

// The terms suggested for a material's `properties` map, or for one
// constitutive model's `parameters` map when `kind` is given.
export function terms(kind) {
  return kind === undefined ? materialProperties : kindParameters(kind);
}

export function term(key, kind) {
  return kind === undefined ? propertyIndex.get(key) : kindParameters(kind).find((p) => p.key === key);
}

// Units to offer for a term. An unrecognised term gets the full list rather
// than an empty one, because a custom property still needs a unit.
export function unitsFor(key, kind) {
  const units = term(key, kind)?.units;
  return units?.length ? units : allUnits;
}

// The unit a term is normally given in — used when a property is first chosen,
// so the common case needs no typing and cannot be mistyped.
export function defaultUnit(key, kind) {
  return term(key, kind)?.units[0];
}

export function label(key, kind) {
  return term(key, kind)?.label;
}

// The first suggested term not already present, for "+ Add". Falls back to a
// placeholder key when a material already carries all of them.
export function nextTerm(existingKeys, kind) {
  const used = new Set(existingKeys);
  return terms(kind).find((candidate) => !used.has(candidate.key));
}
