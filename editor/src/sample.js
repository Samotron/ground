export const sampleDocument = {
  type: "gm.file/1",
  schemaVersion: "0.1.0",
  file: {
    name: "Example route ground model",
    description: "A small two-chainage example for trying the editor.",
    crs: "EPSG:27700",
    verticalDatum: "Ordnance Datum Newlyn",
  },
  materials: [
    {
      materialKey: "MADE_GROUND",
      name: "Made Ground",
      soilClass: "anthropogenic",
      properties: {
        unitWeight: { value: 19, lower: 17, upper: 21, unit: "kN/m3" },
      },
      constitutiveModels: [],
    },
    {
      materialKey: "LONDON_CLAY",
      name: "London Clay",
      soilClass: "clay",
      properties: {
        unitWeight: { value: 20, lower: 19, upper: 21, unit: "kN/m3" },
      },
      constitutiveModels: [],
    },
  ],
  models: [
    {
      modelKey: "CH-100",
      name: "Chainage 100",
      modelType: "1d",
      surfaceLevel: 82.5,
      baseLevel: 62.5,
      x: 384100,
      y: 397200,
      gammaW: 9.81,
      groundwater: { kind: "hydrostatic", depth: 2.5 },
      settings: { allowGaps: false, profileSublayerThickness: 0.5 },
      layers: [
        { topLevel: 82.5, materialKey: "MADE_GROUND", description: "Made Ground" },
        { topLevel: 79.5, materialKey: "LONDON_CLAY", description: "London Clay" },
      ],
    },
    {
      modelKey: "CH-125",
      name: "Chainage 125",
      modelType: "1d",
      surfaceLevel: 81.9,
      baseLevel: 61.9,
      x: 384125,
      y: 397202,
      gammaW: 9.81,
      groundwater: { kind: "hydrostatic", depth: 2.2 },
      settings: { allowGaps: false, profileSublayerThickness: 0.5 },
      layers: [
        { topLevel: 81.9, materialKey: "MADE_GROUND" },
        { topLevel: 79.9, materialKey: "LONDON_CLAY" },
      ],
    },
  ],
};
