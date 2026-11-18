//! The `material` and `sideMaterial` parameters of Extrude & Bevel, Inflate and Rotate: a base color,
//! Roughness and Metallic for the faces of the solid ([`vectorcraft_three_d::Material`]), as the public
//! [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
//! page lists them. `material` covers every face; `sideMaterial` replaces it on the sides (Extrude &
//! Bevel's walls and bevels, Inflate's sides), so the caps and the sides can differ.

use serde_json::Value;
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, FillLayer};
use vectorcraft_three_d::{Facet, MATERIAL_PRESETS, Material, View};

pub(crate) const MATERIAL: &str = "material";
pub(crate) const SIDE_MATERIAL: &str = "sideMaterial";
const KEYS: [&str; 4] = ["preset", "baseColor", "roughness", "metallic"];

/// The materials of one solid effect and the view whose lights shade them.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Materials {
    pub view: View,
    pub front: Option<Material>,
    pub side: Option<Material>,
}

impl Materials {
    /// The material of `face`, if it has one.
    pub fn of(&self, face: &Facet) -> Option<Material> {
        if face.side { self.side.or(self.front) } else { self.front }
    }
}

fn parse(v: &Value, key: &str) -> Result<Option<Material>, String> {
    if v.is_null() {
        return Ok(None);
    }
    let m = v.as_object().ok_or_else(|| format!("{key} must be an object or null"))?;
    if let Some(bad) = m.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown {key} setting {bad}: use preset, baseColor, roughness or metallic"));
    }
    let mut out = match m.get("preset") {
        None => Material::default(),
        Some(p) => p.as_str().and_then(Material::preset).ok_or_else(|| {
            let names: Vec<_> = MATERIAL_PRESETS.iter().map(|p| p.0).collect();
            format!("{key} preset must be one of {}", names.join(", "))
        })?,
    };
    let unit = |name: &str| -> Result<Option<f64>, String> {
        match m.get(name) {
            None => Ok(None),
            Some(v) => v.as_f64().filter(|n| (0.0..=1.0).contains(n)).map(Some).ok_or_else(|| format!("{key} {name} must be a number from 0 to 1")),
        }
    };
    if let Some(r) = unit("roughness")? {
        out.roughness = r;
    }
    if let Some(m) = unit("metallic")? {
        out.metallic = m;
    }
    out.base = match m.get("baseColor") {
        None | Some(Value::Null) => None,
        Some(c) => Some(
            c.as_str()
                .filter(|s| s.len() == 7 && s.starts_with('#') && s.bytes().skip(1).all(|b| b.is_ascii_hexdigit()))
                .and_then(Color::from_hex)
                .ok_or_else(|| format!("{key} baseColor must be an opaque RGB color in #rrggbb form or null"))?
                .to_rgb_uncalibrated()
                .map(f64::from),
        ),
    };
    Ok(Some(out))
}

/// Check the material parameters of `p`.
pub(crate) fn validate(id: &str, p: &Value) -> Result<(), String> {
    for key in [MATERIAL, SIDE_MATERIAL] {
        let Some(v) = p.get(key) else { continue };
        if !matches!(id, super::EXTRUDE | super::INFLATE | super::ROTATE) && !v.is_null() {
            return Err(format!("{key} is taken by Extrude & Bevel, Inflate and Rotate only"));
        }
        parse(v, key)?;
    }
    Ok(())
}

/// The materials effect `id` with parameters `p` gives its solids, `None` when it has none (the
/// shading stays as without materials). Invalid loaded values are left out.
pub(crate) fn materials(id: &str, p: &Value) -> Option<Materials> {
    if !matches!(id, super::EXTRUDE | super::INFLATE | super::ROTATE) {
        return None;
    }
    let get = |key| p.get(key).and_then(|v| parse(v, key).ok()).flatten();
    let (front, side) = (get(MATERIAL), get(SIDE_MATERIAL));
    (front.is_some() || side.is_some()).then(|| Materials { view: super::view_options(p), front, side })
}

fn shaded(c: Color, material: Material, view: &View, normal: [f64; 3]) -> Color {
    let rgb = c.to_rgb().map(f64::from);
    let [r, g, b] = material.shade(view, normal, rgb).map(|v| v as f32);
    Color::rgb(r, g, b)
}

/// The paint of a face in `material`: a solid color (or the base color) shaded as one; a gradient
/// with each stop shaded; a pattern drawn as without a material.
pub(crate) fn fills(paint: &Paint, material: Material, view: &View, face: &Facet, colored: bool) -> Vec<AppearanceItem> {
    if let Some(base) = material.base {
        let [r, g, b] = material.shade(view, face.normal, base).map(|v| v as f32);
        return vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(r, g, b))))];
    }
    if let Some(c) = paint.color() {
        return vec![AppearanceItem::Fill(FillLayer::new(Paint::solid(shaded(c, material, view, face.normal))))];
    }
    if matches!(paint, Paint::Gradient(_)) {
        let mut out = paint.clone();
        vectorcraft_doc::recolor::map_paint(&mut out, &|c| shaded(c, material, view, face.normal));
        if let Paint::Gradient(g) = &mut out {
            g.swatch = None;
        }
        return vec![AppearanceItem::Fill(FillLayer::new(out))];
    }
    crate::lighting::fills(paint, face.illumination, face.brightness, colored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parameters_are_checked_and_presets_fill_in_the_rest() {
        assert!(validate(super::super::EXTRUDE, &json!({"material": {"preset": "polishedMetal"}})).is_ok());
        assert!(validate(super::super::EXTRUDE, &json!({"material": null, "sideMaterial": null})).is_ok());
        for bad in [
            json!({"material": 1}),
            json!({"material": {"preset": "gold"}}),
            json!({"material": {"roughness": 1.5}}),
            json!({"material": {"metallic": "x"}}),
            json!({"material": {"baseColor": "#fff"}}),
            json!({"sideMaterial": {"shine": 1}}),
        ] {
            assert!(validate(super::super::INFLATE, &bad).is_err(), "{bad}");
        }
        assert!(validate("threeD.revolve", &json!({"material": {}})).is_err());
        let m = materials(super::super::ROTATE, &json!({"material": {"preset": "gloss", "metallic": 0.5, "baseColor": "#ff0000"}}));
        let front = m.and_then(|m| m.front);
        assert_eq!(front, Some(Material { base: Some([1.0, 0.0, 0.0]), roughness: 0.15, metallic: 0.5 }));
        assert!(materials(super::super::ROTATE, &json!({})).is_none());
        assert!(materials(super::super::ROTATE, &json!({"material": {"roughness": 9}})).is_none(), "an invalid loaded material is left out");
    }
}
