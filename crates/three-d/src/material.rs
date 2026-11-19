//! Surface materials of the 3D effects: a base color, Roughness and Metallic, as the public
//! [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
//! page lists them (Roughness and Metallic from 0 to 1). The reflection model below and the
//! preset values are VectorCraft's own: a Lambert diffuse term and a Blinn–Phong highlight whose
//! sharpness follows Roughness and whose tint follows Metallic. No Adobe or Substance material
//! is copied.

use crate::{Light, View};

/// A material for the faces of a solid. Without a material a solid keeps its diffuse-only shading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Opaque RGB base color, each channel 0..1. `None` takes the paint of the fill or stroke the
    /// solid is built for.
    pub base: Option<[f64; 3]>,
    /// 0 (polished: a small, bright highlight) to 1 (matte: no highlight).
    pub roughness: f64,
    /// 0 (dielectric: a white highlight over the diffuse color) to 1 (metal: the highlight takes
    /// the base color and the diffuse light fades).
    pub metallic: f64,
}

impl Default for Material {
    /// The default material: the object's own paint, fairly rough, not metallic.
    fn default() -> Self {
        Self { base: None, roughness: 0.6, metallic: 0.0 }
    }
}

/// VectorCraft's own default materials, generated from two numbers each: `(id, roughness, metallic)`.
/// None of them takes a base color, so each keeps the art's paint.
pub const MATERIAL_PRESETS: [(&str, f64, f64); 6] =
    [("default", 0.6, 0.0), ("matte", 1.0, 0.0), ("satin", 0.45, 0.0), ("gloss", 0.15, 0.0), ("brushedMetal", 0.5, 1.0), ("polishedMetal", 0.1, 1.0)];

fn unit(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback }
}

impl Material {
    /// The preset `id` of [`MATERIAL_PRESETS`].
    pub fn preset(id: &str) -> Option<Self> {
        MATERIAL_PRESETS.iter().find(|(name, ..)| *name == id).map(|&(_, roughness, metallic)| Self { base: None, roughness, metallic })
    }

    pub fn sanitized(self) -> Self {
        Self { base: self.base.map(|c| c.map(|v| unit(v, 1.0))), roughness: unit(self.roughness, 0.6), metallic: unit(self.metallic, 0.0) }
    }

    /// The highlight's exponent: 2 when fully rough, 1024 when fully polished.
    fn shininess(self) -> f64 {
        2f64.powf(1.0 + 9.0 * (1.0 - self.roughness))
    }

    /// The color (0..1 RGB) a face of paint color `color` takes under `view`'s lights, with
    /// view-space unit `normal` turned to the viewer. With shading off the face keeps its base color.
    ///
    /// The ambient light and the diffuse light multiply the base color, as without a material;
    /// Metallic lowers the diffuse light to a fifth. Each light that reaches the face adds a
    /// highlight where the face mirrors it toward the eye, `(1 - roughness)²` strong at its peak:
    /// white (at half strength) on a dielectric, the base color on a metal.
    pub fn shade(self, view: &View, normal: [f64; 3], color: [f64; 3]) -> [f64; 3] {
        self.shade_lit(view, normal, color, &[])
    }

    /// [`Material::shade`] with each light's diffuse light and highlight scaled by its entry in
    /// `visibility` (0 in full shadow, 1 unshadowed; a light without an entry is unshadowed), as
    /// the ray-traced render finds them ([`crate::trace`]).
    pub fn shade_lit(self, view: &View, normal: [f64; 3], color: [f64; 3], visibility: &[f64]) -> [f64; 3] {
        let m = self.sanitized();
        let base = m.base.unwrap_or(color).map(|c| unit(c, 0.0));
        if !view.shade {
            return base;
        }
        let n = crate::V3(normal[0], normal[1], normal[2]).unit();
        let (shininess, gloss) = (m.shininess(), (1.0 - m.roughness).powi(2));
        let diffuse_share = 1.0 - 0.8 * m.metallic;
        let tint = base.map(|c| 0.5 * (1.0 - m.metallic) + c * m.metallic);
        let mut light = [unit(view.ambient / 2.0, 0.125) * 2.0; 3];
        let mut highlight = [0.0; 3];
        for (k, l) in view.light_list().into_iter().enumerate() {
            let seen = visibility.get(k).map_or(1.0, |v| unit(*v, 1.0));
            let direct = l.direct(n) * diffuse_share * seen;
            let [x, y, z] = l.direction();
            let reach = n.dot(crate::V3(x, y, z));
            let half = crate::V3(x, y, z + 1.0).unit();
            let peak = if reach > 0.0 && gloss > 0.0 { seen * l.intensity * gloss * n.dot(half).max(0.0).powf(shininess) } else { 0.0 };
            for c in 0..3 {
                light[c] += direct * l.color[c];
                highlight[c] += peak * l.color[c] * tint[c];
            }
        }
        std::array::from_fn(|c| {
            let v = base[c] * light[c].clamp(0.0, 1.0) + highlight[c];
            if v.is_finite() { v.clamp(0.0, 1.0) } else { base[c] }
        })
    }
}

impl View {
    /// The lights of the view: the explicit list, or the single light of the earlier settings.
    pub fn light_list(self) -> Vec<Light> {
        match self.lights {
            Some(lights) => lights.iter().copied().collect(),
            None => vec![
                Light {
                    rotation: self.light_rotation,
                    height: self.light_height,
                    behind: self.light_behind,
                    color: self.light_color,
                    intensity: self.light_intensity,
                    softness: self.light_softness,
                }
                .sanitized(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit_from_front() -> View {
        View { light_rotation: 0.0, light_height: 90.0, light_intensity: 1.0, ambient: 0.0, ..View::default() }
    }

    #[test]
    fn a_rough_dielectric_is_the_diffuse_shading() {
        let view = lit_from_front();
        let matte = Material { base: None, roughness: 1.0, metallic: 0.0 };
        let c = matte.shade(&view, [0.0, 0.0, 1.0], [0.5, 0.25, 1.0]);
        // Fully rough: no highlight; the light straight on lights the color fully.
        assert_eq!(c, [0.5, 0.25, 1.0]);
        let side = matte.shade(&view, [1.0, 0.0, 0.0], [0.5, 0.25, 1.0]);
        assert!(side.iter().all(|c| *c < 1e-12), "a face turned 90° from the light without ambient is dark: {side:?}");
    }

    #[test]
    fn a_polished_surface_has_a_highlight_where_it_mirrors_the_light() {
        let view = lit_from_front();
        let gloss = Material { base: None, roughness: 0.0, metallic: 0.0 };
        let dark = [0.2, 0.2, 0.2];
        let mirror = gloss.shade(&view, [0.0, 0.0, 1.0], dark);
        assert!(mirror.iter().all(|c| *c > 0.6), "{mirror:?}");
        // Turned away from the mirror direction the highlight is gone.
        let n = crate::V3(0.5, 0.0, 1.0).unit();
        let off = gloss.shade(&view, [n.0, n.1, n.2], dark);
        assert!(off.iter().all(|c| *c < 0.25), "{off:?}");
    }

    #[test]
    fn rougher_is_dimmer_and_broader() {
        let view = lit_from_front();
        let n = crate::V3(0.15, 0.0, 1.0).unit();
        let at = |r: f64, n: crate::V3| Material { base: None, roughness: r, metallic: 0.0 }.shade(&view, [n.0, n.1, n.2], [0.0; 3])[0];
        let peak = crate::V3(0.0, 0.0, 1.0);
        assert!(at(0.1, peak) > at(0.5, peak));
        // Slightly off the peak, the broader highlight of the rougher material is brighter.
        assert!(at(0.5, n) > at(0.05, n));
    }

    #[test]
    fn metal_tints_its_highlight_and_darkens_its_diffuse_light() {
        let view = View { ambient: 0.0, ..View::default() };
        let gold = [1.0, 0.75, 0.25];
        let plastic = Material { base: Some(gold), roughness: 0.3, metallic: 0.0 };
        let metal = Material { metallic: 1.0, ..plastic };
        // Lit from the side the highlight misses: metal reflects less diffuse light.
        let n = [0.0, 0.0, 1.0];
        let side = [-0.6, 0.0, 0.8];
        assert!(metal.shade(&view, side, [0.0; 3])[0] < plastic.shade(&view, side, [0.0; 3])[0]);
        // Head-on to a light straight in front, the metal's highlight keeps the base color's hue.
        let front = lit_from_front();
        let c = Material { roughness: 0.0, ..metal }.shade(&front, n, [0.0; 3]);
        assert!(c[0] > c[1] && c[1] > c[2], "{c:?}");
        // The base color replaces the paint.
        assert_eq!(plastic.shade(&View { shade: false, ..view }, n, [0.0; 3]), gold);
    }

    #[test]
    fn colored_lights_and_presets_and_bad_input() {
        let red = Light { color: [1.0, 0.0, 0.0], intensity: 1.0, rotation: 0.0, height: 90.0, ..Light::default() };
        let view = View { lights: Some(crate::Lights::new(&[red]).unwrap()), ambient: 0.0, ..View::default() };
        let c = Material { base: None, roughness: 0.0, metallic: 0.0 }.shade(&view, [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]);
        assert!(c[0] > 0.4 && c[1] == 0.0 && c[2] == 0.0, "a red light gives a red highlight on blue: {c:?}");
        for (id, ..) in MATERIAL_PRESETS {
            assert!(Material::preset(id).is_some());
        }
        assert_eq!(Material::preset("gold"), None);
        let nan = Material { base: Some([f64::NAN; 3]), roughness: f64::NAN, metallic: f64::INFINITY };
        assert!(nan.shade(&view, [f64::NAN; 3], [0.5; 3]).iter().all(|v| v.is_finite()));
    }
}
