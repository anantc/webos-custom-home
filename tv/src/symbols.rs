// Lucide 1.8.0, ISC/MIT. See vendor/lucide/LICENSE. Parsed once at startup.
use anyhow::{Context, Result};
use skia_safe::{Canvas, Color, Paint, Path};
use std::collections::HashMap;
const PATHS: &[(&str, &[&str])] = &[
    ("hdmi-port", &["M22 9a1 1 0 0 0-1-1H3a1 1 0 0 0-1 1v4a1 1 0 0 0 1 1h1l2 2h12l2-2h1a1 1 0 0 0 1-1Z","M7.5 12h9"]),
    ("sun", &["M8 12a4 4 0 1 0 8 0a4 4 0 1 0 -8 0","M12 2v2","M12 20v2","m4.93 4.93 1.41 1.41","m17.66 17.66 1.41 1.41","M2 12h2","M20 12h2","m6.34 17.66-1.41 1.41","m19.07 4.93-1.41 1.41"]),
    ("moon", &["M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401"]),
    ("cloud-sun", &["M12 2v2","m4.93 4.93 1.41 1.41","M20 12h2","m19.07 4.93-1.41 1.41","M15.947 12.65a4 4 0 0 0-5.925-4.128","M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6Z"]),
    ("cloud-moon", &["M13 16a3 3 0 0 1 0 6H7a5 5 0 1 1 4.9-6z","M18.376 14.512a6 6 0 0 0 3.461-4.127c.148-.625-.659-.97-1.248-.714a4 4 0 0 1-5.259-5.26c.255-.589-.09-1.395-.716-1.248a6 6 0 0 0-4.594 5.36"]),
    ("cloud", &["M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"]),
    ("cloud-fog", &["M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242","M16 17H7","M17 21H9"]),
    ("cloud-rain", &["M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242","M16 14v6","M8 14v6","M12 16v6"]),
    ("cloud-snow", &["M4 14.899A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.5 8.242","M8 15h.01","M8 19h.01","M12 17h.01","M12 21h.01","M16 15h.01","M16 19h.01"]),
    ("cloud-lightning", &["M6 16.326A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 .5 8.973","m13 12-3 5h4l-3 5"]),
];
pub struct Symbols(HashMap<&'static str, Vec<Path>>);
impl Symbols {
    pub fn new() -> Result<Self> {
        Ok(Self(
            PATHS
                .iter()
                .map(|(name, paths)| {
                    let paths = paths
                        .iter()
                        .map(|p| Path::from_svg(p).context("Invalid bundled symbol"))
                        .collect::<Result<Vec<_>>>()?;
                    Ok((*name, paths))
                })
                .collect::<Result<_>>()?,
        ))
    }
    pub fn draw(&self, canvas: &Canvas, name: &str, x: f32, y: f32, size: f32, color: Color) {
        let Some(paths) = self.0.get(name) else {
            return;
        };
        let mut paint = Paint::default();
        paint
            .set_color(color)
            .set_anti_alias(true)
            .set_style(skia_safe::paint::Style::Stroke)
            .set_stroke_width(1.6)
            .set_stroke_cap(skia_safe::paint::Cap::Round)
            .set_stroke_join(skia_safe::paint::Join::Round);
        canvas.save();
        canvas.translate((x, y));
        canvas.scale((size / 24.0, size / 24.0));
        for path in paths {
            canvas.draw_path(path, &paint);
        }
        canvas.restore();
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn bundled_paths_parse() {
        assert_eq!(super::Symbols::new().unwrap().0.len(), 10);
    }
}
