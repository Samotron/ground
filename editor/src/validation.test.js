import { describe, expect, it } from "vitest";
import { issueCounts, validateDocument } from "./validation.js";
import { sampleDocument } from "./sample.js";

describe("ground-model validation", () => {
  it("accepts the coherent sample without blocking errors", () => {
    const counts = issueCounts(validateDocument(structuredClone(sampleDocument)));
    expect(counts.error).toBe(0);
    expect(counts.warning).toBeGreaterThan(0);
  });

  it("finds inverted layers and unknown material references", () => {
    const document = structuredClone(sampleDocument);
    document.models[0].layers[1].topLevel = 90;
    document.models[0].layers[1].materialKey = "MISSING";
    const issues = validateDocument(document);
    expect(issues.some((item) => item.severity === "error" && item.message.includes("ordered downwards"))).toBe(true);
    expect(issues.some((item) => item.severity === "error" && item.message.includes("does not define"))).toBe(true);
  });

  it("finds duplicate model and material keys", () => {
    const document = structuredClone(sampleDocument);
    document.models.push(structuredClone(document.models[0]));
    document.materials.push(structuredClone(document.materials[0]));
    const messages = validateDocument(document).map((item) => item.message);
    expect(messages).toContain("duplicate model key 'CH-100'");
    expect(messages).toContain("duplicate material key 'MADE_GROUND'");
  });
});
