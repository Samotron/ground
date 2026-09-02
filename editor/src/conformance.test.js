// Cross-implementation conformance.
//
// assets/conformance.json is read by this suite and by gm-core's. Both must
// agree on it. Two implementations of the same rules drift quietly, and the way
// you find out is an engineer saving a file here that `gm commit` then refuses,
// so the agreement is pinned rather than assumed.
//
// When a rule legitimately changes, regenerate the fixture and change both
// implementations in the same commit. A red test here means they disagree, not
// that the fixture is stale.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { hue } from "./section.js";
import { validateDocument } from "./validation.js";

const fixture = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../assets/conformance.json", import.meta.url)), "utf8"),
);

const sortKey = (issue) => `${issue.severity} ${issue.modelKey ?? ""} ${issue.fieldPath}`;
const normalise = (issues) =>
  issues
    .map((issue) => ({
      severity: issue.severity,
      fieldPath: issue.fieldPath,
      ...(issue.modelKey ? { modelKey: issue.modelKey } : {}),
    }))
    .sort((a, b) => sortKey(a).localeCompare(sortKey(b)));

describe("conformance with gm-core", () => {
  it("gives every material the agreed colour", () => {
    for (const [key, expected] of Object.entries(fixture.hues)) {
      expect(hue(key), `hue for ${JSON.stringify(key)}`).toBe(expected);
    }
  });

  it("reports exactly the agreed issues", () => {
    expect(normalise(validateDocument(fixture.document))).toEqual(normalise(fixture.expectedIssues));
  });

  it("treats a value outside its own bounds as an error, as gm commit does", () => {
    // The divergence this fixture was written to catch: a warning here would
    // let someone save a file that the tool then refuses to commit.
    const outOfBounds = validateDocument(fixture.document).filter((issue) =>
      issue.fieldPath.endsWith(".value"),
    );
    expect(outOfBounds.length).toBeGreaterThan(0);
    for (const issue of outOfBounds) {
      expect(issue.severity, issue.fieldPath).toBe("error");
    }
  });
});
