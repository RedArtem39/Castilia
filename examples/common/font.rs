use castilia::{Font, FontMetrics, css::{FontFace, FontFamily}};
use fontdb::{Database, Family, Query, Style, Weight};
use std::{cell::RefCell, collections::{BTreeMap, HashMap}, rc::Rc};
use ttf_parser::{Face, GlyphId, Tag};

/// A parsed font. Only the tables needed for measuring are read (cmap, hmtx/HVAR, hhea/OS2,
/// kern): no glyph outlines are decoded, so large CJK fonts cost little. Font data lives for the
/// whole process (one page per host process), which lets the parsed face borrow it.
struct Fnt {
    face: Face<'static>,
    data: &'static [u8],
    upem: f32,
    variable: bool,
}

impl Fnt {
    fn parse(data: Vec<u8>, index: u32) -> Option<Fnt> {
        let data: &'static [u8] = Box::leak(data.into_boxed_slice());
        let face = Face::parse(data, index).ok()?;
        let upem = face.units_per_em() as f32;
        if upem <= 0.0 {
            return None;
        }
        let variable = face.is_variable();
        Some(Fnt { face, data, upem, variable })
    }

    fn has(&self, c: char) -> bool {
        self.face.glyph_index(c).is_some_and(|g| g.0 != 0)
    }

    /// The face with the variation axes a browser would apply: `wght` from font-weight and
    /// `opsz` from the font size (`font-optical-sizing: auto`).
    fn instance(&self, weight: u16, size: f32) -> Face<'static> {
        let mut f = self.face.clone();
        for axis in self.face.variation_axes() {
            let value = match &axis.tag.to_bytes() {
                b"wght" => weight as f32,
                b"opsz" => size,
                _ => continue,
            };
            f.set_variation(axis.tag, value.clamp(axis.min_value, axis.max_value));
        }
        f
    }

    fn line(&self, size: f32) -> (f32, f32, f32) {
        let s = size / self.upem;
        (self.face.ascender() as f32 * s, -(self.face.descender() as f32) * s, self.face.line_gap() as f32 * s)
    }
}

fn kern(face: &Face, left: GlyphId, right: GlyphId) -> Option<i16> {
    let table = face.tables().kern?;
    table.subtables.into_iter().filter(|s| s.horizontal && !s.variable).find_map(|s| s.glyphs_kerning(left, right))
}

/// A font in a fallback chain, limited to the characters its `unicode-range` covers.
#[derive(Clone)]
struct Entry {
    font: Rc<Fnt>,
    range: Rc<[(u32, u32)]>,
}

impl Entry {
    fn covers(&self, c: char) -> bool {
        (self.range.is_empty() || self.range.iter().any(|&(a, b)| (a..=b).contains(&(c as u32)))) && self.font.has(c)
    }
}

/// A downloaded `@font-face`.
struct WebFace {
    weight: (u16, u16),
    italic: bool,
    entry: Entry,
    range: Vec<(u32, u32)>,
}

/// Host-side text measurement: web fonts from `@font-face`, system fonts by family name, and
/// generic families (sans-serif, serif, monospace, system-ui), each regular and bold. Characters a
/// font lacks fall through the family list like in a browser.
pub struct SystemFont {
    fonts: [Option<Rc<Fnt>>; 8],
    database: Database,
    /// Lowercase family name → name as the system spells it.
    system_names: HashMap<String, String>,
    named: RefCell<HashMap<(String, u16, bool), Option<Rc<Fnt>>>>,
    web: RefCell<BTreeMap<String, Vec<WebFace>>>,
    chains: RefCell<HashMap<(Option<Rc<str>>, u8, u16, bool), Rc<[Entry]>>>,
    images: RefCell<BTreeMap<String, (f32, f32)>>,
}

/// Font files bigger than this are not loaded by name.
const MAX_NAMED_FONT_BYTES: usize = 40 * 1024 * 1024;

fn load(database: &Database, families: &[Family], weight: u16, italic: bool) -> Option<Fnt> {
    let face = database.query(&Query {
        families,
        weight: Weight(weight),
        style: if italic { Style::Italic } else { Style::Normal },
        ..Default::default()
    })?;
    database
        .with_face_data(face, |data, index| {
            if data.len() > MAX_NAMED_FONT_BYTES {
                return None;
            }
            Fnt::parse(data.to_vec(), index)
        })
        .flatten()
}

fn decode_font(data: &[u8]) -> Option<Vec<u8>> {
    match data.get(..4)? {
        b"wOF2" => wuff::decompress_woff2(data).ok(),
        b"wOFF" => wuff::decompress_woff1(data).ok(),
        _ => Some(data.to_vec()),
    }
}

fn generic_index(name: &str) -> Option<FontFamily> {
    Some(match name {
        "sans-serif" | "ui-sans-serif" => FontFamily::Sans,
        "serif" | "ui-serif" => FontFamily::Serif,
        "monospace" | "ui-monospace" => FontFamily::Monospace,
        "system-ui" => FontFamily::System,
        // Chromium honours these only on macOS
        "-apple-system" | "blinkmacsystemfont" if cfg!(target_os = "macos") => FontFamily::System,
        _ => return None,
    })
}

impl SystemFont {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let mut database = Database::new();
        database.load_system_fonts();
        let sans = [Family::Name("Liberation Sans"), Family::Name("Arial"), Family::Name("Inter"), Family::Name("DejaVu Sans"), Family::SansSerif];
        let serif = [Family::Name("Liberation Serif"), Family::Name("Times New Roman"), Family::Name("DejaVu Serif"), Family::Serif];
        let mono = [Family::Name("DejaVu Sans Mono"), Family::Name("Liberation Mono"), Family::Name("Courier New"), Family::Monospace];
        let system = [Family::Name("Segoe UI"), Family::Name("Inter"), Family::Name("Cantarell"), Family::Name("Liberation Sans"), Family::SansSerif];
        let fonts = [
            load(&database, &serif, 400, false),
            load(&database, &serif, 700, false),
            load(&database, &sans, 400, false),
            load(&database, &sans, 700, false),
            load(&database, &mono, 400, false),
            load(&database, &mono, 700, false),
            load(&database, &system, 400, false),
            load(&database, &system, 700, false),
        ]
        .map(|f| f.map(Rc::new));
        if fonts.iter().all(|f| f.is_none()) {
            return Err("no system fonts found".into());
        }
        let mut system_names = HashMap::new();
        for face in database.faces() {
            for (name, _) in &face.families {
                system_names.entry(name.to_lowercase()).or_insert_with(|| name.clone());
            }
        }
        Ok(Self {
            fonts,
            database,
            system_names,
            named: RefCell::new(HashMap::new()),
            web: RefCell::new(BTreeMap::new()),
            chains: RefCell::new(HashMap::new()),
            images: RefCell::new(BTreeMap::new()),
        })
    }

    pub fn set_image_size(&self, src: &str, size: (f32, f32)) {
        self.images.borrow_mut().insert(src.to_string(), size);
    }

    fn generic(&self, family: FontFamily, weight: u16) -> Rc<Fnt> {
        let base = match family {
            FontFamily::Serif => 0,
            FontFamily::Sans => 2,
            FontFamily::Monospace => 4,
            FontFamily::System => 6,
        };
        let idx = base + (weight >= 600) as usize;
        self.fonts[idx]
            .as_ref()
            .or(self.fonts[base].as_ref())
            .or(self.fonts[2].as_ref())
            .or_else(|| self.fonts.iter().flatten().next())
            .expect("at least one font")
            .clone()
    }

    fn named_font(&self, name: &str, weight: u16, italic: bool) -> Option<Rc<Fnt>> {
        let real = self.system_names.get(name)?;
        let key = (name.to_string(), weight, italic);
        if let Some(f) = self.named.borrow().get(&key) {
            return f.clone();
        }
        let f = load(&self.database, &[Family::Name(real)], weight, italic).map(Rc::new);
        self.named.borrow_mut().insert(key, f.clone());
        f
    }

    /// CSS font matching within one web family: the faces (all unicode-range subsets) of the best
    /// weight and style.
    fn web_entries(faces: &[WebFace], weight: u16, italic: bool, out: &mut Vec<Entry>) {
        let style_ok: Vec<&WebFace> = {
            let same: Vec<&WebFace> = faces.iter().filter(|f| f.italic == italic).collect();
            if same.is_empty() { faces.iter().collect() } else { same }
        };
        let distance = |f: &WebFace| -> i32 {
            let (lo, hi) = f.weight;
            if (lo..=hi).contains(&weight) {
                return 0;
            }
            let d = if weight < lo { lo - weight } else { weight - hi } as i32;
            // heavier faces win for bold requests, lighter ones for light requests
            let heavier = lo > weight;
            if (weight >= 500) == heavier { d } else { d + 1000 }
        };
        let Some(best) = style_ok.iter().map(|f| distance(f)).min() else { return };
        out.extend(style_ok.iter().filter(|f| distance(f) == best).map(|f| f.entry.clone()));
    }

    fn chain(&self, font: &Font) -> Rc<[Entry]> {
        let key = (font.names.clone(), font.family as u8, font.weight, font.italic);
        if let Some(c) = self.chains.borrow().get(&key) {
            return c.clone();
        }
        let mut out = Vec::new();
        let none: Rc<[(u32, u32)]> = Rc::from(Vec::new());
        if let Some(names) = &font.names {
            let web = self.web.borrow();
            for name in names.split(',').take(8) {
                if let Some(faces) = web.get(name) {
                    Self::web_entries(faces, font.weight, font.italic, &mut out);
                } else if let Some(generic) = generic_index(name) {
                    out.push(Entry { font: self.generic(generic, font.weight), range: none.clone() });
                } else if let Some(f) = self.named_font(name, font.weight, font.italic) {
                    out.push(Entry { font: f, range: none.clone() });
                }
            }
        }
        out.push(Entry { font: self.generic(font.family, font.weight), range: none });
        let chain: Rc<[Entry]> = Rc::from(out);
        self.chains.borrow_mut().insert(key, chain.clone());
        chain
    }

    /// The primary font: the first one in the chain that has a space glyph (Blink's rule).
    fn primary(&self, font: &Font) -> Rc<Fnt> {
        let chain = self.chain(font);
        chain.iter().find(|e| e.covers(' ')).unwrap_or(&chain[chain.len() - 1]).font.clone()
    }

    /// `@font-face` rules with the downloaded fonts embedded, for SVG output.
    pub fn svg_font_css(&self) -> String {
        let mut css = String::new();
        for (family, faces) in self.web.borrow().iter() {
            for f in faces {
                let range = if f.range.is_empty() {
                    String::new()
                } else {
                    format!(";unicode-range:{}", f.range.iter().map(|(a, b)| format!("U+{a:X}-{b:X}")).collect::<Vec<_>>().join(","))
                };
                css.push_str(&format!(
                    "@font-face{{font-family:'{}';font-weight:{} {};font-style:{};src:url(data:font/ttf;base64,{}){}}}\n",
                    family.replace('\'', ""),
                    f.weight.0,
                    f.weight.1,
                    if f.italic { "italic" } else { "normal" },
                    base64(f.entry.font.data),
                    range
                ));
            }
        }
        css
    }
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn width_with(chain: &[Entry], text: &str, font: &Font) -> f32 {
    let size = font.size;
    // variable fonts are measured at the instance the browser renders
    let instances: Vec<Option<Face<'static>>> = chain.iter().map(|e| e.font.variable.then(|| e.font.instance(font.weight, size))).collect();
    let mut width = 0.0;
    let mut previous: Option<(GlyphId, usize)> = None;
    for character in text.chars() {
        let Some(i) = chain.iter().position(|e| e.covers(character)) else {
            if !character.is_control() {
                // No font has it: the browser picks a system fallback (emoji are about 1.2em wide).
                let cp = character as u32;
                width += match cp {
                    0xFE00..=0xFE0F | 0x200B..=0x200D | 0x20E3 | 0x1F3FB..=0x1F3FF => 0.0,
                    0x2E80..=0x9FFF | 0xAC00..=0xD7A3 | 0xFF00..=0xFF60 | 0x3000..=0x303F => size,
                    0x2190..=0x2BFF | 0x1F000..=0x1FAFF | 0x2600..=0x27BF => size * 1.248,
                    _ => size * 0.6,
                };
            }
            previous = None;
            continue;
        };
        let f = &chain[i].font;
        let face = instances[i].as_ref().unwrap_or(&f.face);
        let Some(g) = face.glyph_index(character) else { continue };
        let scale = size / f.upem;
        if let Some((left, j)) = previous
            && j == i
        {
            width += kern(face, left, g).unwrap_or(0) as f32 * scale;
        }
        width += face.glyph_hor_advance(g).unwrap_or(0) as f32 * scale;
        previous = Some((g, i));
    }
    width
}

fn plain(size: f32) -> Font {
    Font { size, weight: 400, italic: false, family: FontFamily::Sans, names: None }
}

impl FontMetrics for SystemFont {
    fn text_width(&self, text: &str, size: f32) -> f32 {
        self.text_width_font(text, &plain(size))
    }

    fn text_width_font(&self, text: &str, font: &Font) -> f32 {
        width_with(&self.chain(font), text, font)
    }

    fn line_height(&self, size: f32) -> f32 {
        self.line_height_font(&plain(size))
    }

    fn line_height_font(&self, font: &Font) -> f32 {
        // Blink: lround(ascent) + lround(descent) + lround(line gap) for the primary font.
        let (a, d, g) = self.primary(font).line(font.size);
        a.round() + d.round() + g.round()
    }

    fn ascent(&self, font: &Font) -> f32 {
        self.primary(font).line(font.size).0.round()
    }

    fn descent(&self, font: &Font) -> f32 {
        self.primary(font).line(font.size).1.round()
    }

    fn add_font_face(&self, face: &FontFace, data: &[u8]) -> bool {
        let Some(sfnt) = decode_font(data) else { return false };
        let Some(font) = Fnt::parse(sfnt, 0) else { return false };
        let range: Rc<[(u32, u32)]> = Rc::from(face.unicode_range.clone());
        self.web.borrow_mut().entry(face.family.clone()).or_default().push(WebFace {
            weight: face.weight,
            italic: face.italic,
            entry: Entry { font: Rc::new(font), range },
            range: face.unicode_range.clone(),
        });
        self.chains.borrow_mut().clear();
        true
    }

    fn image_size(&self, src: &str) -> Option<(f32, f32)> {
        self.images.borrow().get(src).copied()
    }
}
