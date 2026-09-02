//! An SVG section through a 1D model.
//!
//! This is the reason the web UI exists. A table of levels is something you have
//! to decode; a drawn section is something an engineer reads at a glance, and
//! errors in a succession — an inverted boundary, a water table below the base,
//! a stratum far thinner than the others — are obvious in a picture and easy to
//! miss in a list.

use super::page::escape;
use gm_core::model::{GroundModel, Groundwater, Material};
use std::collections::BTreeMap;
use std::fmt::Write;

const WIDTH: f64 = 460.0;
const COLUMN_LEFT: f64 = 92.0;
const COLUMN_WIDTH: f64 = 132.0;
const TOP: f64 = 24.0;
const MIN_HEIGHT: f64 = 320.0;
const PER_METRE: f64 = 13.0;

/// A stable, well-spread hue for a material key.
///
/// Deterministic so a given stratum keeps its colour across every model, every
/// page and every session — an engineer scanning a route should be able to
/// recognise London Clay by sight without reading the label each time.
fn hue(key: &str) -> u32 {
    // FNV-1a, then take the golden-angle multiple so adjacent keys land far
    // apart on the colour wheel rather than in adjacent shades.
    let mut hash: u32 = 2_166_136_261;
    for byte in key.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash.wrapping_mul(137) % 360).max(1)
}

/// Draw the succession. Returns an `<svg>` element, self-contained apart from
/// the stylesheet in [`super::page`].
pub fn draw(model: &GroundModel, materials: &BTreeMap<String, Material>) -> String {
    let (Some(surface), Some(base)) = (model.surface_level, model.base_level) else {
        return String::from(
            "<p class=\"note\">This model has no surface or base level, so it cannot be drawn. \
             Set both and it will appear here.</p>",
        );
    };
    if model.layers.is_empty() || base >= surface {
        return String::from("<p class=\"note\">Nothing to draw yet.</p>");
    }

    let extent = surface - base;
    let height = (extent * PER_METRE).clamp(MIN_HEIGHT, 900.0);
    let scale = height / extent;
    // Level -> y in the drawing.
    let y_of = |level: f64| TOP + (surface - level) * scale;
    let total = height + TOP * 2.0 + 16.0;

    let mut svg = String::new();
    let _ = write!(
        svg,
        r#"<svg class="section" viewBox="0 0 {WIDTH} {total:.0}" width="{WIDTH}" height="{total:.0}" role="img" aria-label="Section through {}">"#,
        escape(&model.model_key)
    );

    for (i, layer) in model.layers.iter().enumerate() {
        let top_y = y_of(layer.top_level);
        let layer_base = model.layer_base(i).unwrap_or(base);
        let box_height = (y_of(layer_base) - top_y).max(1.0);
        let h = hue(&layer.material_key);

        let _ = write!(
            svg,
            r#"<rect class="layer" x="{COLUMN_LEFT}" y="{top_y:.1}" width="{COLUMN_WIDTH}" height="{box_height:.1}" style="--fill:hsl({h} 34% 76%);--fill-dark:hsl({h} 26% 30%)"/>"#
        );

        // Boundary line and the level that goes with it.
        let _ = write!(
            svg,
            r#"<line class="boundary" x1="{COLUMN_LEFT}" y1="{top_y:.1}" x2="{:.1}" y2="{top_y:.1}"/>"#,
            COLUMN_LEFT + COLUMN_WIDTH
        );
        let _ = write!(
            svg,
            r#"<text class="level" x="{:.1}" y="{:.1}">{:.2}</text>"#,
            COLUMN_LEFT - 8.0,
            top_y + 4.0,
            layer.top_level
        );
        let _ = write!(
            svg,
            r#"<text class="depth" x="{:.1}" y="{:.1}">{:.2} m</text>"#,
            COLUMN_LEFT - 8.0,
            top_y + 17.0,
            surface - layer.top_level
        );

        // Name the stratum inside its box when it is tall enough to hold text,
        // and beside it when it is not, so thin layers stay labelled.
        let name = materials
            .get(&layer.material_key)
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| layer.material_key.clone());
        if box_height >= 20.0 {
            let _ = write!(
                svg,
                r#"<text class="stratum" x="{:.1}" y="{:.1}">{}</text>"#,
                COLUMN_LEFT + COLUMN_WIDTH / 2.0,
                top_y + box_height / 2.0 + 4.0,
                escape(&name)
            );
        } else {
            let _ = write!(
                svg,
                r#"<text class="stratum-outside" x="{:.1}" y="{:.1}">{}</text>"#,
                COLUMN_LEFT + COLUMN_WIDTH + 10.0,
                top_y + box_height / 2.0 + 4.0,
                escape(&name)
            );
        }
    }

    // Base of model.
    let base_y = y_of(base);
    let _ = write!(
        svg,
        r#"<line class="boundary base" x1="{COLUMN_LEFT}" y1="{base_y:.1}" x2="{:.1}" y2="{base_y:.1}"/>"#,
        COLUMN_LEFT + COLUMN_WIDTH
    );
    let _ = write!(
        svg,
        r#"<text class="level" x="{:.1}" y="{:.1}">{base:.2}</text>"#,
        COLUMN_LEFT - 8.0,
        base_y + 4.0
    );
    let _ = write!(
        svg,
        r#"<text class="depth" x="{:.1}" y="{:.1}">{:.2} m</text>"#,
        COLUMN_LEFT - 8.0,
        base_y + 17.0,
        extent
    );

    // Ground surface hatch, so the top of the model reads as ground rather than
    // as just another boundary.
    let _ = write!(
        svg,
        r#"<line class="ground" x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}"/>"#,
        COLUMN_LEFT - 14.0,
        TOP,
        COLUMN_LEFT + COLUMN_WIDTH + 14.0,
        TOP
    );

    if let Groundwater::Hydrostatic { depth } = model.groundwater {
        let level = surface - depth;
        if level <= surface && level >= base {
            let y = y_of(level);
            let x = COLUMN_LEFT + COLUMN_WIDTH;
            let _ = write!(
                svg,
                r#"<line class="water" x1="{:.1}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}"/>"#,
                COLUMN_LEFT - 6.0,
                x + 6.0
            );
            // The conventional inverted triangle marking a water table.
            let _ = write!(
                svg,
                r#"<polygon class="water-mark" points="{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}"/>"#,
                x + 12.0,
                y - 6.0,
                x + 24.0,
                y - 6.0,
                x + 18.0,
                y + 4.0
            );
            let _ = write!(
                svg,
                r#"<text class="water-label" x="{:.1}" y="{:.1}">{depth:.2} m</text>"#,
                x + 30.0,
                y + 4.0
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;
    use gm_core::model::Layer;

    fn layer(top: f64, key: &str) -> Layer {
        Layer {
            top_level: top,
            material_key: key.into(),
            description: None,
            source: None,
            generated_from_profile: false,
            metadata: None,
        }
    }

    fn two_layer_model() -> GroundModel {
        let mut m = GroundModel::new("CH-100");
        m.surface_level = Some(82.5);
        m.base_level = Some(62.5);
        m.groundwater = Groundwater::Hydrostatic { depth: 2.5 };
        m.layers = vec![layer(82.5, "MADE_GROUND"), layer(79.5, "LONDON_CLAY")];
        m
    }

    #[test]
    fn every_layer_gets_a_box_and_the_section_is_closed_at_the_base() {
        let svg = draw(&two_layer_model(), &BTreeMap::new());
        assert_eq!(svg.matches("class=\"layer\"").count(), 2);
        assert!(
            svg.contains("class=\"boundary base\""),
            "base must be drawn"
        );
        assert!(svg.contains(">62.50<"), "base level should be labelled");
        assert!(svg.contains(">82.50<"), "surface level should be labelled");
    }

    #[test]
    fn boxes_are_scaled_to_thickness_and_stacked_without_gaps() {
        let svg = draw(&two_layer_model(), &BTreeMap::new());
        // 20 m over 320 px is 16 px/m: 3 m of Made Ground is 48 px, and the
        // clay must start exactly where it ends.
        assert!(
            svg.contains(r#"y="24.0" width="132" height="48.0""#),
            "got {svg}"
        );
        assert!(
            svg.contains(r#"y="72.0" width="132" height="272.0""#),
            "got {svg}"
        );
    }

    #[test]
    fn a_model_without_extent_says_so_rather_than_drawing_nonsense() {
        let mut m = two_layer_model();
        m.base_level = None;
        assert!(draw(&m, &BTreeMap::new()).contains("cannot be drawn"));
    }

    #[test]
    fn the_water_table_is_drawn_only_when_it_is_inside_the_model() {
        let inside = draw(&two_layer_model(), &BTreeMap::new());
        assert!(inside.contains("class=\"water\""));

        let mut deep = two_layer_model();
        deep.groundwater = Groundwater::Hydrostatic { depth: 40.0 };
        assert!(
            !draw(&deep, &BTreeMap::new()).contains("class=\"water\""),
            "a water table below the base must not be drawn off the bottom"
        );
    }

    #[test]
    fn a_material_keeps_its_colour_everywhere() {
        assert_eq!(hue("LONDON_CLAY"), hue("LONDON_CLAY"));
        assert_ne!(hue("LONDON_CLAY"), hue("MADE_GROUND"));
    }

    #[test]
    fn material_names_are_escaped_in_the_drawing() {
        let mut materials = BTreeMap::new();
        let mut m = gm_core::Material {
            material_key: "MADE_GROUND".into(),
            name: Some("<script>".into()),
            description: None,
            soil_class: None,
            properties: BTreeMap::new(),
            constitutive_models: vec![],
            provenance: None,
            metadata: None,
        };
        m.name = Some("<script>".into());
        materials.insert("MADE_GROUND".to_string(), m);

        let svg = draw(&two_layer_model(), &materials);
        assert!(!svg.contains("<script>"), "material names must be escaped");
        assert!(svg.contains("&lt;script&gt;"));
    }
}
