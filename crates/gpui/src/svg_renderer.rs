use crate::{
    AssetSource, Bounds, DevicePixels, Hsla, ImageCacheError, ImageLoadLimits, IsZero, ObjectFit,
    Pixels, RenderImage, Result, Rgba, SharedString, Size, point, px, swap_rgba_pa_to_bgra,
};
use image::Frame;
use resvg::tiny_skia::Pixmap;
use smallvec::SmallVec;
use std::{
    hash::Hash,
    sync::{Arc, LazyLock, OnceLock},
};

#[cfg(target_os = "macos")]
const EMOJI_FONT_FAMILIES: &[&str] = &["Apple Color Emoji", ".AppleColorEmojiUI"];

#[cfg(target_os = "windows")]
const EMOJI_FONT_FAMILIES: &[&str] = &["Segoe UI Emoji", "Segoe UI Symbol"];

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const EMOJI_FONT_FAMILIES: &[&str] = &[
    "Noto Color Emoji",
    "Emoji One",
    "Twitter Color Emoji",
    "JoyPixels",
];

#[cfg(not(any(
    target_os = "macos",
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
)))]
const EMOJI_FONT_FAMILIES: &[&str] = &[];

fn is_emoji_presentation(c: char) -> bool {
    static EMOJI_PRESENTATION_REGEX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new("\\p{Emoji_Presentation}").unwrap());
    let mut buf = [0u8; 4];
    EMOJI_PRESENTATION_REGEX.is_match(c.encode_utf8(&mut buf))
}

fn font_has_char(db: &usvg::fontdb::Database, id: usvg::fontdb::ID, ch: char) -> bool {
    db.with_face_data(id, |font_data, face_index| {
        ttf_parser::Face::parse(font_data, face_index)
            .ok()
            .and_then(|face| face.glyph_index(ch))
            .is_some()
    })
    .unwrap_or(false)
}

fn select_emoji_font(
    ch: char,
    fonts: &[usvg::fontdb::ID],
    db: &usvg::fontdb::Database,
    families: &[&str],
) -> Option<usvg::fontdb::ID> {
    for family_name in families {
        let query = usvg::fontdb::Query {
            families: &[usvg::fontdb::Family::Name(family_name)],
            weight: usvg::fontdb::Weight(400),
            stretch: usvg::fontdb::Stretch::Normal,
            style: usvg::fontdb::Style::Normal,
        };

        let Some(id) = db.query(&query) else {
            continue;
        };

        if fonts.contains(&id) || !font_has_char(db, id, ch) {
            continue;
        }

        return Some(id);
    }

    None
}

/// When rendering SVGs, we render them at twice the size to get a higher-quality result.
pub const SMOOTH_SVG_SCALE_FACTOR: f32 = 2.;

#[derive(Clone, PartialEq, Hash, Eq)]
#[expect(missing_docs)]
pub struct RenderSvgParams {
    pub path: SharedString,
    pub size: Size<DevicePixels>,
}

/// Parameters used to rasterize and cache a colored SVG.
#[derive(Clone, PartialEq, Hash, Eq)]
pub struct RenderColorSvgParams {
    /// The asset path of the SVG.
    pub path: SharedString,
    /// The target raster size.
    pub size: Size<DevicePixels>,
    /// The target element size in logical pixels, before display scaling and smoothing.
    pub logical_size: Size<Pixels>,
    /// How the SVG's intrinsic dimensions fit within the target element.
    pub object_fit: ObjectFit,
    /// The CSS `currentColor` value.
    pub current_color: Option<Hsla>,
    /// An optional override for shape fills.
    pub fill_color: Option<Hsla>,
    /// An optional override for SVG text and tspan elements.
    pub text_color: Option<Hsla>,
}

#[derive(Clone)]
/// A struct holding everything necessary to render SVGs.
pub struct SvgRenderer {
    asset_source: Arc<dyn AssetSource>,
    enriched_fontdb: Arc<OnceLock<Arc<usvg::fontdb::Database>>>,
}

/// The size in which to render the SVG.
pub enum SvgSize {
    /// An absolute size in device pixels.
    Size(Size<DevicePixels>),
    /// A scaling factor to apply to the size provided by the SVG.
    ScaleFactor(f32),
}

impl SvgRenderer {
    /// Creates a new SVG renderer with the provided asset source.
    pub fn new(asset_source: Arc<dyn AssetSource>) -> Self {
        // Build the enriched font DB lazily on first SVG render rather than
        // eagerly at construction time. This avoids the expensive deep-clone
        // of the system font database for code paths that never render SVGs
        // (e.g. tests).
        let enriched_fontdb = Arc::new(OnceLock::new());

        Self {
            asset_source,
            enriched_fontdb,
        }
    }

    fn options(&self, style_sheet: Option<String>) -> usvg::Options<'static> {
        static SYSTEM_FONT_DB: LazyLock<Arc<usvg::fontdb::Database>> = LazyLock::new(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        });

        let default_font_resolver = usvg::FontResolver::default_font_selector();
        let font_resolver = Box::new({
            let asset_source = self.asset_source.clone();
            let enriched_fontdb = self.enriched_fontdb.clone();
            move |font: &usvg::Font, db: &mut Arc<usvg::fontdb::Database>| {
                if db.is_empty() {
                    let fontdb = enriched_fontdb.get_or_init(|| {
                        let mut db = (**SYSTEM_FONT_DB).clone();
                        load_bundled_fonts(&*asset_source, &mut db);
                        fix_generic_font_families(&mut db);
                        Arc::new(db)
                    });
                    *db = fontdb.clone();
                }
                if let Some(id) = default_font_resolver(font, db) {
                    return Some(id);
                }
                // fontdb doesn't recognize CSS system font keywords like "system-ui"
                // or "ui-sans-serif", so fall back to sans-serif before any face.
                let sans_query = usvg::fontdb::Query {
                    families: &[usvg::fontdb::Family::SansSerif],
                    ..Default::default()
                };
                db.query(&sans_query)
                    .or_else(|| db.faces().next().map(|f| f.id))
            }
        });
        let default_fallback_selection = usvg::FontResolver::default_fallback_selector();
        let fallback_selection = Box::new(
            move |ch: char, fonts: &[usvg::fontdb::ID], db: &mut Arc<usvg::fontdb::Database>| {
                if is_emoji_presentation(ch) {
                    if let Some(id) = select_emoji_font(ch, fonts, db.as_ref(), EMOJI_FONT_FAMILIES)
                    {
                        return Some(id);
                    }
                }

                default_fallback_selection(ch, fonts, db)
            },
        );
        usvg::Options {
            font_resolver: usvg::FontResolver {
                select_font: font_resolver,
                select_fallback: fallback_selection,
            },
            style_sheet,
            ..Default::default()
        }
    }

    /// Renders the given bytes into an image buffer.
    pub fn render_single_frame(
        &self,
        bytes: &[u8],
        scale_factor: f32,
    ) -> Result<Arc<RenderImage>, usvg::Error> {
        self.render_pixmap(
            bytes,
            SvgSize::ScaleFactor(scale_factor * SMOOTH_SVG_SCALE_FACTOR),
            None,
        )
        .map(pixmap_to_image)
    }

    /// Renders an SVG with input and output pixel limits.
    ///
    /// Dimensions include the smoothing scale factor. Parsing and embedded SVG
    /// resources are not covered by the decoded pixel budget.
    pub fn render_single_frame_with_limits(
        &self,
        bytes: &[u8],
        scale_factor: f32,
        limits: ImageLoadLimits,
    ) -> Result<Arc<RenderImage>, ImageCacheError> {
        limits.check_input(bytes.len() as u64)?;
        self.render_pixmap_checked(
            bytes,
            SvgSize::ScaleFactor(scale_factor * SMOOTH_SVG_SCALE_FACTOR),
            None,
            |width, height| limits.check_frame(width, height).map(|_| ()),
        )
        .map(pixmap_to_image)
    }

    pub(crate) fn render_alpha_mask(
        &self,
        params: &RenderSvgParams,
        bytes: Option<&[u8]>,
    ) -> Result<Option<(Size<DevicePixels>, Vec<u8>)>> {
        anyhow::ensure!(!params.size.is_zero(), "can't render at a zero size");

        let render_pixmap = |bytes| {
            let pixmap = self.render_pixmap(bytes, SvgSize::Size(params.size), None)?;

            // Convert the pixmap's pixels into an alpha mask.
            let size = Size::new(
                DevicePixels(pixmap.width() as i32),
                DevicePixels(pixmap.height() as i32),
            );
            let alpha_mask = pixmap
                .pixels()
                .iter()
                .map(|p| p.alpha())
                .collect::<Vec<_>>();

            Ok(Some((size, alpha_mask)))
        };

        if let Some(bytes) = bytes {
            render_pixmap(bytes)
        } else if let Some(bytes) = self.asset_source.load(&params.path)? {
            render_pixmap(&bytes)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn render_color_image(
        &self,
        params: &RenderColorSvgParams,
        bytes: Option<&[u8]>,
    ) -> Result<Option<(Size<DevicePixels>, Vec<u8>)>> {
        anyhow::ensure!(!params.size.is_zero(), "can't render at a zero size");
        let style_sheet = color_svg_style_sheet(params);
        let render_pixmap = |bytes| {
            let options = self.options((!style_sheet.is_empty()).then(|| style_sheet.clone()));
            let tree = usvg::Tree::from_data(bytes, &options)?;
            let pixmap = render_fitted_color_svg(&tree, params)?;
            let size = Size::new(
                DevicePixels(pixmap.width() as i32),
                DevicePixels(pixmap.height() as i32),
            );
            let mut bytes = pixmap.take();
            for pixel in bytes.chunks_exact_mut(4) {
                swap_rgba_pa_to_bgra(pixel);
            }
            Ok(Some((size, bytes)))
        };

        if let Some(bytes) = bytes {
            render_pixmap(bytes)
        } else if let Some(bytes) = self.asset_source.load(&params.path)? {
            render_pixmap(&bytes)
        } else {
            Ok(None)
        }
    }

    fn render_pixmap(
        &self,
        bytes: &[u8],
        size: SvgSize,
        style_sheet: Option<String>,
    ) -> Result<Pixmap, usvg::Error> {
        self.render_pixmap_checked(bytes, size, style_sheet, |_, _| Ok(()))
    }

    fn render_pixmap_checked<E: From<usvg::Error>>(
        &self,
        bytes: &[u8],
        size: SvgSize,
        style_sheet: Option<String>,
        check_dimensions: impl FnOnce(u32, u32) -> Result<(), E>,
    ) -> Result<Pixmap, E> {
        // Cap the size of the rendered pixmap to avoid texture allocation panics
        // Related issue: #56466
        const MAX_SIZE: f32 = 8192.0;

        let options = self.options(style_sheet);
        let tree = usvg::Tree::from_data(bytes, &options)?;
        let svg_size = tree.size();
        let mut scale = match size {
            SvgSize::Size(size) => size.width.0 as f32 / svg_size.width(),
            SvgSize::ScaleFactor(scale) => scale,
        };

        let width = svg_size.width() * scale;
        if width > MAX_SIZE {
            log::warn!("Attempted to render pixmap where width ({width}) > MAX_SIZE ({MAX_SIZE})");
            scale *= MAX_SIZE / width;
        }
        let height = svg_size.height() * scale;
        if height > MAX_SIZE {
            log::warn!(
                "Attempted to render pixmap where height ({height}) > MAX_SIZE ({MAX_SIZE})"
            );
            scale *= MAX_SIZE / height;
        }

        // Render the SVG to a pixmap with the specified width and height.
        let width = (svg_size.width() * scale) as u32;
        let height = (svg_size.height() * scale) as u32;
        check_dimensions(width, height)?;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).ok_or(usvg::Error::InvalidSize)?;

        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);

        resvg::render(&tree, transform, &mut pixmap.as_mut());

        Ok(pixmap)
    }
}

// Rasterize into the element's viewport so fitting and clipping happen on the same
// worker as parsing. No intrinsic-size lookup or additional cache is needed on paint.
fn render_fitted_color_svg(
    tree: &usvg::Tree,
    params: &RenderColorSvgParams,
) -> Result<Pixmap, usvg::Error> {
    let logical_size = params.logical_size;
    if params.size.width.0 <= 0
        || params.size.height.0 <= 0
        || !logical_size.width.0.is_finite()
        || !logical_size.height.0.is_finite()
        || logical_size.width <= px(0.)
        || logical_size.height <= px(0.)
    {
        return Err(usvg::Error::InvalidSize);
    }
    let source = tree.size();
    let fitted = params.object_fit.get_bounds_for_size(
        Bounds::new(point(px(0.), px(0.)), logical_size),
        Size::new(px(source.width()), px(source.height())),
    );
    // Keep the existing texture limit without changing the displayed logical size.
    let reduction = (8192. / params.size.width.0 as f32)
        .min(8192. / params.size.height.0 as f32)
        .min(1.);
    let width = ((params.size.width.0 as f32 * reduction).round() as u32).max(1);
    let height = ((params.size.height.0 as f32 * reduction).round() as u32).max(1);
    let raster_x = width as f32 / logical_size.width.0;
    let raster_y = height as f32 / logical_size.height.0;
    let transform = resvg::tiny_skia::Transform::from_row(
        fitted.size.width.0 / source.width() * raster_x,
        0.,
        0.,
        fitted.size.height.0 / source.height() * raster_y,
        (logical_size.width.0 - fitted.size.width.0) / 2. * raster_x,
        (logical_size.height.0 - fitted.size.height.0) / 2. * raster_y,
    );
    let mut pixmap = Pixmap::new(width, height).ok_or(usvg::Error::InvalidSize)?;
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

fn pixmap_to_image(pixmap: Pixmap) -> Arc<RenderImage> {
    let mut buffer =
        image::ImageBuffer::from_raw(pixmap.width(), pixmap.height(), pixmap.take()).unwrap();
    for pixel in buffer.chunks_exact_mut(4) {
        swap_rgba_pa_to_bgra(pixel);
    }
    let mut image = RenderImage::new(SmallVec::from_const([Frame::new(buffer)]));
    image.scale_factor = SMOOTH_SVG_SCALE_FACTOR;
    Arc::new(image)
}

fn color_svg_style_sheet(params: &RenderColorSvgParams) -> String {
    fn css_color(color: Hsla) -> String {
        format!("#{:08x}", u32::from(Rgba::from(color)))
    }

    let mut style_sheet = String::new();
    if let Some(color) = params.current_color {
        style_sheet.push_str(&format!(
            "svg, svg * {{ color: {} !important; }}\n",
            css_color(color)
        ));
    }
    if let Some(color) = params.fill_color {
        style_sheet.push_str(&format!(
            "path, rect, circle, ellipse, polygon, polyline {{ fill: {} !important; }}\n",
            css_color(color)
        ));
    }
    if let Some(color) = params.text_color {
        let color = css_color(color);
        style_sheet.push_str(&format!(
            "text, tspan {{ color: {color} !important; fill: {color} !important; }}\n"
        ));
    }
    style_sheet
}

fn load_bundled_fonts(asset_source: &dyn AssetSource, db: &mut usvg::fontdb::Database) {
    let font_paths = [
        "fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf",
        "fonts/lilex/Lilex-Regular.ttf",
    ];
    for path in font_paths {
        match asset_source.load(path) {
            Ok(Some(data)) => db.load_font_data(data.into_owned()),
            Ok(None) => log::warn!("Bundled font not found: {path}"),
            Err(error) => log::warn!("Failed to load bundled font {path}: {error}"),
        }
    }
}

// fontdb defaults generic families to Microsoft fonts ("Arial", "Times New Roman")
// which aren't installed on most Linux systems. fontconfig normally overrides these,
// but when it fails the defaults remain and all generic family queries return None.
fn fix_generic_font_families(db: &mut usvg::fontdb::Database) {
    use usvg::fontdb::{Family, Query};

    let families_and_fallbacks: &[(Family<'_>, &str)] = &[
        (Family::SansSerif, "IBM Plex Sans"),
        // No serif font bundled; use sans-serif as best available fallback.
        (Family::Serif, "IBM Plex Sans"),
        (Family::Monospace, "Lilex"),
        (Family::Cursive, "IBM Plex Sans"),
        (Family::Fantasy, "IBM Plex Sans"),
    ];

    for (family, fallback_name) in families_and_fallbacks {
        let query = Query {
            families: &[*family],
            ..Default::default()
        };
        if db.query(&query).is_none() {
            match family {
                Family::SansSerif => db.set_sans_serif_family(*fallback_name),
                Family::Serif => db.set_serif_family(*fallback_name),
                Family::Monospace => db.set_monospace_family(*fallback_name),
                Family::Cursive => db.set_cursive_family(*fallback_name),
                Family::Fantasy => db.set_fantasy_family(*fallback_name),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{blue, green, red};
    use usvg::fontdb::{Database, Family, Query};

    const IBM_PLEX_REGULAR: &[u8] =
        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf");
    const LILEX_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/lilex/Lilex-Regular.ttf");

    fn db_with_bundled_fonts() -> Database {
        let mut db = Database::new();
        db.load_font_data(IBM_PLEX_REGULAR.to_vec());
        db.load_font_data(LILEX_REGULAR.to_vec());
        db
    }

    #[test]
    fn text_with_split_glyph_clusters_in_mixed_fonts_does_not_panic() {
        let mut db = Database::new();
        db.load_font_data(IBM_PLEX_REGULAR.to_vec());
        db.load_font_data(LILEX_REGULAR.to_vec());
        let options = usvg::Options {
            fontdb: std::sync::Arc::new(db),
            ..Default::default()
        };

        // A base letter followed by a stack of combining marks. Under HarfBuzz's
        // default cluster merging every mark glyph shares the base's byte index,
        // which is the "glyph splitting" condition that triggered the panic. The
        // chunk must use two different fonts so the buggy merge path runs.
        let zalgo = "e\u{0301}\u{0302}\u{0303}\u{0304}\u{0306}\u{0307}\u{0308}\u{030a}";
        let svg = format!(
            r#"<svg viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg"><text font-family="Lilex" font-size="32">{zalgo}<tspan font-family="IBM Plex Sans">{zalgo}</tspan></text></svg>"#
        );

        // Before the fix this aborts via panic with a message like
        // "removal index (is 5) should be < len (is 5)".
        usvg::Tree::from_data(svg.as_bytes(), &options)
            .expect("SVG with mixed-font text should parse");
    }

    #[test]
    fn test_is_emoji_presentation() {
        let cases = [
            ("a", false),
            ("Z", false),
            ("1", false),
            ("#", false),
            ("*", false),
            ("漢", false),
            ("中", false),
            ("カ", false),
            ("©", false),
            ("♥", false),
            ("😀", true),
            ("✅", true),
            ("🇺🇸", true),
            // SVG fallback is not cluster-aware yet
            ("©️", false),
            ("♥️", false),
            ("1️⃣", false),
        ];
        for (s, expected) in cases {
            assert_eq!(
                is_emoji_presentation(s.chars().next().unwrap()),
                expected,
                "for char {:?}",
                s
            );
        }
    }

    #[test]
    fn fix_generic_font_families_sets_all_families() {
        let mut db = db_with_bundled_fonts();
        fix_generic_font_families(&mut db);

        let families = [
            Family::SansSerif,
            Family::Serif,
            Family::Monospace,
            Family::Cursive,
            Family::Fantasy,
        ];

        for family in families {
            let query = Query {
                families: &[family],
                ..Default::default()
            };
            assert!(
                db.query(&query).is_some(),
                "Expected generic family {family:?} to resolve after fix_generic_font_families"
            );
        }
    }

    #[test]
    fn test_select_emoji_font_skips_family_without_glyph() {
        let mut db = db_with_bundled_fonts();

        let ibm_plex_sans = db
            .query(&usvg::fontdb::Query {
                families: &[usvg::fontdb::Family::Name("IBM Plex Sans")],
                weight: usvg::fontdb::Weight(400),
                stretch: usvg::fontdb::Stretch::Normal,
                style: usvg::fontdb::Style::Normal,
            })
            .unwrap();
        let lilex = db
            .query(&usvg::fontdb::Query {
                families: &[usvg::fontdb::Family::Name("Lilex")],
                weight: usvg::fontdb::Weight(400),
                stretch: usvg::fontdb::Stretch::Normal,
                style: usvg::fontdb::Style::Normal,
            })
            .unwrap();
        let selected = select_emoji_font('│', &[], &db, &["IBM Plex Sans", "Lilex"]).unwrap();

        assert_eq!(selected, lilex);
        assert!(!font_has_char(&db, ibm_plex_sans, '│'));
        assert!(font_has_char(&db, selected, '│'));
    }

    #[test]
    fn fix_generic_font_families_monospace_resolves_to_lilex() {
        let mut db = db_with_bundled_fonts();
        fix_generic_font_families(&mut db);

        let query = Query {
            families: &[Family::Monospace],
            ..Default::default()
        };
        let id = db.query(&query).expect("Monospace should resolve");
        let face = db.face(id).expect("Face should exist");
        assert!(
            face.families.iter().any(|(name, _)| name.contains("Lilex")),
            "Monospace should map to Lilex, got {:?}",
            face.families
        );
    }

    fn color_svg_params() -> RenderColorSvgParams {
        RenderColorSvgParams {
            path: "test.svg".into(),
            size: Size::new(DevicePixels(20), DevicePixels(10)),
            logical_size: Size::new(px(20.), px(10.)),
            object_fit: ObjectFit::Contain,
            current_color: None,
            fill_color: None,
            text_color: None,
        }
    }

    fn pixel_at(bytes: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        bytes[(y * width + x) * 4..][..4].try_into().unwrap()
    }

    fn opaque_bounds(bytes: &[u8], width: usize) -> (usize, usize, usize, usize) {
        let mut left = usize::MAX;
        let mut top = usize::MAX;
        let mut right = 0;
        let mut bottom = 0;
        for (index, pixel) in bytes.chunks_exact(4).enumerate() {
            if pixel[3] > 127 {
                left = left.min(index % width);
                top = top.min(index / width);
                right = right.max(index % width + 1);
                bottom = bottom.max(index / width + 1);
            }
        }
        (left, top, right - left, bottom - top)
    }

    #[test]
    fn color_svg_object_fit_rasterizes_centered_at_all_aspect_ratios() {
        for (source_w, source_h, fit, expected) in [
            (24, 24, ObjectFit::Contain, (150, 0, 600, 600)),
            (200, 100, ObjectFit::Contain, (0, 75, 900, 450)),
            (100, 200, ObjectFit::Contain, (300, 0, 300, 600)),
            (200, 100, ObjectFit::Cover, (0, 0, 900, 600)),
            (200, 100, ObjectFit::Fill, (0, 0, 900, 600)),
            (200, 100, ObjectFit::ScaleDown, (350, 250, 200, 100)),
            (200, 100, ObjectFit::None, (350, 250, 200, 100)),
            (1800, 900, ObjectFit::ScaleDown, (0, 75, 900, 450)),
            (1800, 900, ObjectFit::None, (0, 0, 900, 600)),
        ] {
            // A viewBox-only source exercises usvg's intrinsic-size resolution too.
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {source_w} {source_h}"><rect width="{source_w}" height="{source_h}" fill="red"/></svg>"#
            );
            let params = RenderColorSvgParams {
                size: Size::new(DevicePixels(900), DevicePixels(600)),
                logical_size: Size::new(px(900.), px(600.)),
                object_fit: fit,
                ..color_svg_params()
            };
            let (size, bytes) = SvgRenderer::new(Arc::new(()))
                .render_color_image(&params, Some(svg.as_bytes()))
                .unwrap()
                .unwrap();
            assert_eq!(size, params.size);
            assert_eq!(
                opaque_bounds(&bytes, 900),
                expected,
                "{fit:?}, {source_w}x{source_h}"
            );
        }
    }

    #[test]
    fn color_svg_cover_crops_both_sides_and_fill_stretches() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
            <rect width="200" height="100" fill="lime"/>
            <rect width="25" height="100" fill="red"/>
            <rect x="175" width="25" height="100" fill="blue"/>
        </svg>"#;
        let renderer = SvgRenderer::new(Arc::new(()));
        for fit in [ObjectFit::Cover, ObjectFit::Fill, ObjectFit::Contain] {
            let params = RenderColorSvgParams {
                size: Size::new(DevicePixels(900), DevicePixels(600)),
                logical_size: Size::new(px(900.), px(600.)),
                object_fit: fit,
                ..color_svg_params()
            };
            let (_, bytes) = renderer
                .render_color_image(&params, Some(svg))
                .unwrap()
                .unwrap();
            // Cover scales to 1200x600 and removes 150 px on each side, exactly
            // cropping the red and blue 25-unit source stripes.
            if fit == ObjectFit::Cover {
                assert_eq!(pixel_at(&bytes, 900, 1, 300), [0, 255, 0, 255]);
                assert_eq!(pixel_at(&bytes, 900, 898, 300), [0, 255, 0, 255]);
            } else {
                assert_eq!(pixel_at(&bytes, 900, 1, 300), [0, 0, 255, 255]);
                assert_eq!(pixel_at(&bytes, 900, 898, 300), [255, 0, 0, 255]);
            }
            assert_eq!(
                pixel_at(&bytes, 900, 450, 1)[3],
                if fit == ObjectFit::Contain { 0 } else { 255 }
            );
        }
    }

    #[test]
    fn color_svg_intrinsic_size_respects_fractional_dimensions_and_raster_scale() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20.5" height="10.5"><rect width="20.5" height="10.5"/></svg>"#;
        for fit in [ObjectFit::None, ObjectFit::ScaleDown] {
            // 2x display scale and 2x smoothing: intrinsic size stays 20.5x10.5 logical px.
            let params = RenderColorSvgParams {
                size: Size::new(DevicePixels(362), DevicePixels(242)),
                logical_size: Size::new(px(90.5), px(60.5)),
                object_fit: fit,
                ..color_svg_params()
            };
            let (_, bytes) = SvgRenderer::new(Arc::new(()))
                .render_color_image(&params, Some(svg))
                .unwrap()
                .unwrap();
            assert_eq!(opaque_bounds(&bytes, 362), (140, 100, 82, 42));
        }
    }

    #[test]
    fn color_svg_preserves_source_colors_and_applies_current_color() {
        let svg = br##"<svg width="20" height="10" xmlns="http://www.w3.org/2000/svg">
            <rect width="10" height="10" fill="#ff0000"/>
            <rect x="10" width="10" height="10" fill="currentColor" style="color: #00ff00"/>
        </svg>"##;
        let renderer = SvgRenderer::new(Arc::new(()));
        let mut params = color_svg_params();
        params.current_color = Some(blue());

        let (_, bytes) = renderer
            .render_color_image(&params, Some(svg))
            .unwrap()
            .unwrap();

        // Polychrome atlas pixels use unpremultiplied BGRA byte order.
        assert_eq!(pixel_at(&bytes, 20, 5, 5), [0, 0, 255, 255]);
        assert_eq!(pixel_at(&bytes, 20, 15, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn color_svg_fill_override_replaces_attributes_and_inline_styles() {
        let svg = br##"<svg width="20" height="10" xmlns="http://www.w3.org/2000/svg">
            <rect width="10" height="10" fill="#ff0000"/>
            <rect x="10" width="10" height="10" style="fill: #0000ff"/>
        </svg>"##;
        let renderer = SvgRenderer::new(Arc::new(()));
        let mut params = color_svg_params();
        params.fill_color = Some(green());

        let (_, bytes) = renderer
            .render_color_image(&params, Some(svg))
            .unwrap()
            .unwrap();

        assert_eq!(pixel_at(&bytes, 20, 5, 5), [0, 127, 0, 255]);
        assert_eq!(pixel_at(&bytes, 20, 15, 5), [0, 127, 0, 255]);
    }

    #[test]
    fn color_svg_style_sheet_keeps_text_color_separate_from_fill_override() {
        let mut params = color_svg_params();
        params.current_color = Some(red());
        params.fill_color = Some(green());
        params.text_color = Some(blue());

        let style_sheet = color_svg_style_sheet(&params);

        assert!(style_sheet.contains("svg, svg * { color: #ff0000ff !important; }"));
        assert!(style_sheet.contains("path, rect, circle, ellipse, polygon, polyline"));
        assert!(style_sheet.contains("fill: #007f00ff !important;"));
        assert!(
            style_sheet.contains(
                "text, tspan { color: #0000ffff !important; fill: #0000ffff !important; }"
            )
        );
    }
}
