use castilia::FontMetrics;
use fontdb::{Database, Family, Query};
use fontdue::{Font, FontSettings};

/// Host-only measurement using the system sans-serif font.
pub struct SystemFont {
    font: Font,
}

impl SystemFont {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut database = Database::new();
        database.load_system_fonts();
        let face = database
            .query(&Query {
                families: &[Family::SansSerif],
                ..Default::default()
            })
            .ok_or("no system sans-serif font found")?;
        let font = database
            .with_face_data(face, |data, index| {
                Font::from_bytes(
                    data,
                    FontSettings {
                        collection_index: index,
                        ..Default::default()
                    },
                )
            })
            .ok_or("could not read system font")??;
        Ok(Self { font })
    }
}

impl FontMetrics for SystemFont {
    fn text_width(&self, text: &str, size: f32) -> f32 {
        let mut width = 0.0;
        let mut previous = None;
        for character in text.chars() {
            if let Some(left) = previous {
                width += self
                    .font
                    .horizontal_kern(left, character, size)
                    .unwrap_or(0.0);
            }
            width += self.font.metrics(character, size).advance_width;
            previous = Some(character);
        }
        width
    }

    fn line_height(&self, size: f32) -> f32 {
        self.font
            .horizontal_line_metrics(size)
            .map_or(size * 1.2, |metrics| metrics.new_line_size)
    }
}
