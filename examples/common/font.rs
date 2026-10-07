use castilia::{Font, FontMetrics, css::FontFamily};
use fontdb::{Database, Family, Query, Style, Weight};
use fontdue::{Font as FdFont, FontSettings};
use std::{cell::RefCell, collections::BTreeMap};

/// Host-only measurement using system fonts (sans-serif, serif and monospace, regular and bold).
pub struct SystemFont {
    fonts: [Option<FdFont>; 6],
    images: RefCell<BTreeMap<String, (f32, f32)>>,
}

fn load(database: &Database, families: &[Family], bold: bool) -> Option<FdFont> {
    let face = database.query(&Query {
        families,
        weight: if bold { Weight::BOLD } else { Weight::NORMAL },
        style: Style::Normal,
        ..Default::default()
    })?;
    database
        .with_face_data(face, |data, index| {
            FdFont::from_bytes(data, FontSettings { collection_index: index, ..Default::default() }).ok()
        })
        .flatten()
}

impl SystemFont {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut database = Database::new();
        database.load_system_fonts();
        let sans = [Family::Name("Liberation Sans"), Family::Name("Arial"), Family::Name("Inter"), Family::Name("DejaVu Sans"), Family::SansSerif];
        let serif = [Family::Name("Liberation Serif"), Family::Name("Times New Roman"), Family::Name("DejaVu Serif"), Family::Serif];
        let mono = [Family::Name("DejaVu Sans Mono"), Family::Name("Liberation Mono"), Family::Name("Courier New"), Family::Monospace];
        let fonts = [
            load(&database, &serif, false),
            load(&database, &serif, true),
            load(&database, &sans, false),
            load(&database, &sans, true),
            load(&database, &mono, false),
            load(&database, &mono, true),
        ];
        if fonts[2].is_none() && fonts.iter().all(|f| f.is_none()) {
            return Err("no system fonts found".into());
        }
        Ok(Self { fonts, images: RefCell::new(BTreeMap::new()) })
    }

    pub fn set_image_size(&self, src: &str, size: (f32, f32)) {
        self.images.borrow_mut().insert(src.to_string(), size);
    }

    fn pick(&self, font: &Font) -> &FdFont {
        let base = match font.family {
            FontFamily::Serif => 0,
            FontFamily::Sans => 2,
            FontFamily::Monospace => 4,
        };
        let idx = base + (font.weight >= 600) as usize;
        self.fonts[idx]
            .as_ref()
            .or(self.fonts[base].as_ref())
            .or(self.fonts[2].as_ref())
            .or_else(|| self.fonts.iter().flatten().next())
            .expect("at least one font")
    }
}

fn width_with(font: &FdFont, text: &str, size: f32) -> f32 {
    let mut width = 0.0;
    let mut previous = None;
    for character in text.chars() {
        if let Some(left) = previous {
            width += font.horizontal_kern(left, character, size).unwrap_or(0.0);
        }
        width += font.metrics(character, size).advance_width;
        previous = Some(character);
    }
    width
}

impl FontMetrics for SystemFont {
    fn text_width(&self, text: &str, size: f32) -> f32 {
        self.text_width_font(text, &Font { size, weight: 400, italic: false, family: FontFamily::Sans })
    }

    fn text_width_font(&self, text: &str, font: &Font) -> f32 {
        width_with(self.pick(font), text, font.size)
    }

    fn line_height(&self, size: f32) -> f32 {
        self.line_height_font(&Font { size, weight: 400, italic: false, family: FontFamily::Sans })
    }

    fn line_height_font(&self, font: &Font) -> f32 {
        // Blink: lround(ascent) + lround(descent) + lround(line gap) for the primary font.
        self.pick(font).horizontal_line_metrics(font.size).map_or(font.size * 1.2, |m| m.ascent.round() + (-m.descent).round() + m.line_gap.round())
    }

    fn ascent(&self, font: &Font) -> f32 {
        self.pick(font).horizontal_line_metrics(font.size).map_or(font.size * 0.9, |m| m.ascent.round())
    }

    fn descent(&self, font: &Font) -> f32 {
        self.pick(font).horizontal_line_metrics(font.size).map_or(font.size * 0.2, |m| (-m.descent).round())
    }

    fn image_size(&self, src: &str) -> Option<(f32, f32)> {
        self.images.borrow().get(src).copied()
    }
}
