use crate::{FontId, GlyphId, Pixels, PlatformTextSystem, Point, SharedString, Size, point, px};
use collections::FxHashMap;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use smallvec::SmallVec;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Range,
    sync::Arc,
};

use super::LineWrapper;

/// A laid out and styled line of text
#[derive(Default, Debug)]
pub struct LineLayout {
    /// The font size for this line
    pub font_size: Pixels,
    /// The width of the line
    pub width: Pixels,
    /// The ascent of the line
    pub ascent: Pixels,
    /// The descent of the line
    pub descent: Pixels,
    /// The shaped runs that make up this line
    pub runs: Vec<ShapedRun>,
    /// The length of the line in utf-8 bytes
    pub len: usize,
}

/// A run of text that has been shaped .
#[derive(Debug, Clone)]
pub struct ShapedRun {
    /// The font id for this run
    pub font_id: FontId,
    /// The glyphs that make up this run
    pub glyphs: Vec<ShapedGlyph>,
}

/// A single glyph, ready to paint.
#[derive(Clone, Debug)]
pub struct ShapedGlyph {
    /// The ID for this glyph, as determined by the text system.
    pub id: GlyphId,

    /// The position of this glyph in its containing line.
    pub position: Point<Pixels>,

    /// UTF-8 start of the shaping cluster containing this glyph.
    pub index: usize,

    /// Exclusive UTF-8 end of the shaping cluster containing this glyph.
    pub cluster_end: usize,

    /// Horizontal advance, excluding ink bearings and offsets.
    pub advance: Pixels,

    /// Whether the shaping cluster runs from right to left.
    pub is_rtl: bool,

    /// Whether this glyph is an emoji
    pub is_emoji: bool,
}

/// Which adjacent shaping cluster owns a caret at a logical text boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaretAffinity {
    /// The trailing edge of the preceding logical cluster.
    Upstream,
    /// The leading edge of the following logical cluster.
    #[default]
    Downstream,
}

/// A UTF-8 caret position, including its side at directional and wrap boundaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextCaret {
    /// UTF-8 byte offset in the line.
    pub index: usize,
    /// Side of a boundary that can have two visual positions.
    pub affinity: CaretAffinity,
}

impl From<usize> for TextCaret {
    fn from(index: usize) -> Self {
        Self {
            index,
            affinity: CaretAffinity::Downstream,
        }
    }
}

struct Cluster {
    range: Range<usize>,
    left: Pixels,
    right: Pixels,
    rtl: bool,
}

impl Cluster {
    fn leading(&self) -> Pixels {
        if self.rtl { self.right } else { self.left }
    }
    fn trailing(&self) -> Pixels {
        if self.rtl { self.left } else { self.right }
    }
    fn distance(&self, x: Pixels) -> Pixels {
        (self.left - x).max(x - self.right).max(px(0.))
    }
    fn closest_caret(&self, x: Pixels) -> TextCaret {
        if (x - self.leading()).abs() <= (x - self.trailing()).abs() {
            self.range.start.into()
        } else {
            TextCaret {
                index: self.range.end,
                affinity: CaretAffinity::Upstream,
            }
        }
    }
}

impl LineLayout {
    fn clusters(&self) -> impl Iterator<Item = Cluster> + '_ {
        let mut glyphs = self.runs.iter().flat_map(|run| &run.glyphs).peekable();
        std::iter::from_fn(move || {
            let glyph = glyphs.next()?;
            let mut cluster = Cluster {
                range: glyph.index..glyph.cluster_end,
                left: glyph.position.x,
                right: glyph.position.x + glyph.advance,
                rtl: glyph.is_rtl,
            };
            while glyphs.peek().is_some_and(|next| next.index == glyph.index) {
                let next = glyphs.next().unwrap();
                if next.advance > px(0.) {
                    if cluster.left == cluster.right {
                        cluster.left = next.position.x;
                        cluster.right = next.position.x + next.advance;
                    } else {
                        cluster.left = cluster.left.min(next.position.x);
                        cluster.right = cluster.right.max(next.position.x + next.advance);
                    }
                }
                cluster.range.end = cluster.range.end.max(next.cluster_end);
            }
            Some(cluster)
        })
    }

    /// The index for the character at the given x coordinate
    pub fn index_for_x(&self, x: Pixels) -> Option<usize> {
        if x >= self.width {
            None
        } else {
            self.clusters()
                .min_by_key(|cluster| (cluster.distance(x), x >= cluster.right))
                .map(|cluster| cluster.range.start)
                .or(Some(0))
        }
    }

    /// closest_index_for_x returns the character boundary closest to the given x coordinate
    /// (e.g. to handle aligning up/down arrow keys)
    pub fn closest_index_for_x(&self, x: Pixels) -> usize {
        self.caret_for_x(x).index
    }

    /// Closest caret, preserving the side of a directional boundary.
    pub fn caret_for_x(&self, x: Pixels) -> TextCaret {
        self.clusters()
            .min_by_key(|cluster| (cluster.distance(x), x >= cluster.right))
            .map_or(TextCaret::default(), |cluster| cluster.closest_caret(x))
    }

    /// The x position of the character at the given index
    pub fn x_for_index(&self, index: usize) -> Pixels {
        self.x_for_caret(index.into())
    }

    fn cluster_for_caret(&self, caret: TextCaret) -> Option<Cluster> {
        if caret.affinity == CaretAffinity::Upstream {
            if let Some(cluster) = self
                .clusters()
                .find(|cluster| cluster.range.end == caret.index)
            {
                return Some(cluster);
            }
        }
        self.clusters()
            .find(|cluster| cluster.range.contains(&caret.index))
            .or_else(|| {
                self.clusters()
                    .filter(|cluster| cluster.range.end <= caret.index)
                    .max_by_key(|cluster| cluster.range.end)
            })
    }

    /// X position of a caret, including its side at a directional boundary.
    pub fn x_for_caret(&self, caret: TextCaret) -> Pixels {
        let index = caret.index;
        self.cluster_for_caret(caret).map_or(px(0.), |cluster| {
            if index >= cluster.range.end {
                cluster.trailing()
            } else {
                cluster.leading()
            }
        })
    }

    /// The corresponding Font at the given index
    pub fn font_id_for_index(&self, index: usize) -> Option<FontId> {
        for run in &self.runs {
            for glyph in &run.glyphs {
                if glyph.index <= index && index < glyph.cluster_end {
                    return Some(run.font_id);
                }
            }
        }
        None
    }

    /// Visual intervals covered by a logical UTF-8 selection. Intersected shaping
    /// clusters are selected whole; disjoint intervals remain separate.
    pub fn selection_ranges(&self, selected: Range<usize>) -> Vec<Range<Pixels>> {
        if selected.is_empty() {
            return Vec::new();
        }
        let mut ranges: Vec<_> = self
            .clusters()
            .filter(|cluster| {
                cluster.range.start < selected.end && selected.start < cluster.range.end
            })
            .map(|cluster| cluster.left..cluster.right)
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<Pixels>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }

    fn compute_wrap_boundaries(
        &self,
        text: &str,
        wrap_width: Pixels,
        max_lines: Option<usize>,
    ) -> SmallVec<[WrapBoundary; 1]> {
        let mut boundaries = SmallVec::new();
        let mut first_non_whitespace_ix = None;
        let mut last_candidate_ix = None;
        let mut last_candidate_x = px(0.);
        let mut last_boundary = WrapBoundary {
            run_ix: 0,
            glyph_ix: 0,
        };
        let mut last_boundary_x = px(0.);
        let mut prev_ch = '\0';
        let mut previous_cluster = None;
        let mut glyphs = self
            .runs
            .iter()
            .enumerate()
            .flat_map(move |(run_ix, run)| {
                run.glyphs.iter().enumerate().map(move |(glyph_ix, glyph)| {
                    let character = text[glyph.index..].chars().next().unwrap();
                    (
                        WrapBoundary { run_ix, glyph_ix },
                        character,
                        glyph.position.x,
                    )
                })
            })
            .filter(|(boundary, _, _)| {
                let index = self.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index;
                previous_cluster.replace(index) != Some(index)
            })
            .peekable();

        while let Some((boundary, ch, x)) = glyphs.next() {
            if ch == '\n' {
                continue;
            }

            // Here is very similar to `LineWrapper::wrap_line` to determine text wrapping,
            // but there are some differences, so we have to duplicate the code here.
            if LineWrapper::is_word_char(ch) {
                if prev_ch == ' ' && ch != ' ' && first_non_whitespace_ix.is_some() {
                    last_candidate_ix = Some(boundary);
                    last_candidate_x = x;
                }
            } else {
                if ch != ' ' && first_non_whitespace_ix.is_some() {
                    last_candidate_ix = Some(boundary);
                    last_candidate_x = x;
                }
            }

            if ch != ' ' && first_non_whitespace_ix.is_none() {
                first_non_whitespace_ix = Some(boundary);
            }

            let next_x = glyphs.peek().map_or(self.width, |(_, _, x)| *x);
            let width = next_x - last_boundary_x;

            if width > wrap_width && boundary > last_boundary {
                // When used line_clamp, we should limit the number of lines.
                if let Some(max_lines) = max_lines
                    && boundaries.len() >= max_lines.saturating_sub(1)
                {
                    break;
                }

                if let Some(last_candidate_ix) = last_candidate_ix.take() {
                    last_boundary = last_candidate_ix;
                    last_boundary_x = last_candidate_x;
                } else {
                    last_boundary = boundary;
                    last_boundary_x = x;
                }
                boundaries.push(last_boundary);
            }
            prev_ch = ch;
        }

        boundaries
    }
}

/// A line of text that has been wrapped to fit a given width
#[derive(Default, Debug)]
pub struct WrappedLineLayout {
    /// The line layout, pre-wrapping.
    pub unwrapped_layout: Arc<LineLayout>,

    /// The boundaries at which the line was wrapped
    pub wrap_boundaries: SmallVec<[WrapBoundary; 1]>,

    /// The width of the line, if it was wrapped
    pub wrap_width: Option<Pixels>,
}

/// A boundary at which a line was wrapped
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WrapBoundary {
    /// The index in the run just before the line was wrapped
    pub run_ix: usize,
    /// The index of the glyph just before the line was wrapped
    pub glyph_ix: usize,
}

impl WrappedLineLayout {
    /// The length of the underlying text, in utf8 bytes.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.unwrapped_layout.len
    }

    /// The width of this line, in pixels, whether or not it was wrapped.
    pub fn width(&self) -> Pixels {
        self.wrap_width
            .unwrap_or(Pixels::MAX)
            .min(self.unwrapped_layout.width)
    }

    /// The size of the whole wrapped text, for the given line_height.
    /// can span multiple lines if there are multiple wrap boundaries.
    pub fn size(&self, line_height: Pixels) -> Size<Pixels> {
        Size {
            width: self.width(),
            height: line_height * (self.wrap_boundaries.len() + 1),
        }
    }

    /// The ascent of a line in this layout
    pub fn ascent(&self) -> Pixels {
        self.unwrapped_layout.ascent
    }

    /// The descent of a line in this layout
    pub fn descent(&self) -> Pixels {
        self.unwrapped_layout.descent
    }

    /// The wrap boundaries in this layout
    pub fn wrap_boundaries(&self) -> &[WrapBoundary] {
        &self.wrap_boundaries
    }

    /// The font size of this layout
    pub fn font_size(&self) -> Pixels {
        self.unwrapped_layout.font_size
    }

    /// The runs in this layout, sans wrapping
    pub fn runs(&self) -> &[ShapedRun] {
        &self.unwrapped_layout.runs
    }

    /// The index corresponding to a given position in this layout for the given line height.
    ///
    /// See also [`Self::closest_index_for_position`].
    pub fn index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, false)
    }

    /// The closest index to a given position in this layout for the given line height.
    ///
    /// Closest means the character boundary closest to the given position.
    ///
    /// See also [`LineLayout::closest_index_for_x`].
    pub fn closest_index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> Result<usize, usize> {
        self._index_for_position(position, line_height, true)
    }

    /// Horizontal extent of a displayed row in the unwrapped coordinate space.
    pub fn row_extent(&self, row: usize) -> Option<Range<Pixels>> {
        if row > self.wrap_boundaries.len() {
            return None;
        }
        let boundary_x = |boundary: &WrapBoundary| {
            self.runs()[boundary.run_ix].glyphs[boundary.glyph_ix]
                .position
                .x
        };
        let start = if row == 0 {
            px(0.)
        } else {
            boundary_x(&self.wrap_boundaries[row - 1])
        };
        let end = self
            .wrap_boundaries
            .get(row)
            .map(boundary_x)
            .unwrap_or(self.unwrapped_layout.width);
        Some(start..end)
    }

    /// Logical envelope of the clusters painted on a displayed row.
    pub fn row_range(&self, row: usize) -> Option<Range<usize>> {
        let extent = self.row_extent(row)?;
        let mut range: Option<Range<usize>> = None;
        for cluster in self
            .unwrapped_layout
            .clusters()
            .filter(|cluster| cluster.right > extent.start && cluster.left < extent.end)
        {
            if let Some(range) = &mut range {
                range.start = range.start.min(cluster.range.start);
                range.end = range.end.max(cluster.range.end);
            } else {
                range = Some(cluster.range);
            }
        }
        Some(range.unwrap_or(0..0))
    }

    pub(crate) fn row_text_ranges(&self, row: usize) -> Vec<Range<usize>> {
        let Some(extent) = self.row_extent(row) else {
            return Vec::new();
        };
        let mut ranges: Vec<_> = self
            .unwrapped_layout
            .clusters()
            .filter(|cluster| cluster.right > extent.start && cluster.left < extent.end)
            .map(|cluster| cluster.range)
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut()
                && range.start <= last.end
            {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }

    fn _index_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
        closest: bool,
    ) -> Result<usize, usize> {
        let row = (position.y / line_height) as usize;
        let Some(extent) = self.row_extent(row) else {
            return Err(self.len());
        };
        let x = position.x + extent.start;
        let cluster = self
            .unwrapped_layout
            .clusters()
            .filter(|cluster| cluster.right > extent.start && cluster.left < extent.end)
            .min_by_key(|cluster| (cluster.distance(x), x >= cluster.right));
        let index = cluster.map_or(0, |cluster| {
            if closest || x < extent.start || x >= extent.end {
                cluster.closest_caret(x).index
            } else {
                cluster.range.start
            }
        });
        if position.y < px(0.) || x < extent.start || x >= extent.end {
            Err(index)
        } else {
            Ok(index)
        }
    }

    /// Closest caret in displayed coordinates, preserving boundary affinity.
    pub fn closest_caret_for_position(
        &self,
        position: Point<Pixels>,
        line_height: Pixels,
    ) -> TextCaret {
        let row = ((position.y / line_height).max(0.) as usize).min(self.wrap_boundaries.len());
        let extent = self.row_extent(row).unwrap();
        let x = position.x + extent.start;
        self.unwrapped_layout
            .clusters()
            .filter(|cluster| cluster.right > extent.start && cluster.left < extent.end)
            .min_by_key(|cluster| (cluster.distance(x), x >= cluster.right))
            .map_or(TextCaret::default(), |cluster| cluster.closest_caret(x))
    }

    /// Pixel position of a logical UTF-8 index, on the following cluster's side.
    pub fn position_for_index(&self, index: usize, line_height: Pixels) -> Option<Point<Pixels>> {
        self.position_for_caret(index.into(), line_height)
    }

    /// Pixel position of a caret, preserving directional and wrap affinity.
    pub fn position_for_caret(
        &self,
        caret: TextCaret,
        line_height: Pixels,
    ) -> Option<Point<Pixels>> {
        if caret.index > self.len() {
            return None;
        }
        let x = self.unwrapped_layout.x_for_caret(caret);
        let cluster = self.unwrapped_layout.cluster_for_caret(caret);
        let probe = cluster.map_or(x, |cluster| (cluster.left + cluster.right) / 2.);
        let row = self.wrap_boundaries.partition_point(|boundary| {
            self.runs()[boundary.run_ix].glyphs[boundary.glyph_ix]
                .position
                .x
                <= probe
        });
        let extent = self.row_extent(row)?;
        Some(point(x - extent.start, line_height * row))
    }

    /// Shaping-cluster caret edges in displayed coordinates. Directional and
    /// wrap boundaries retain both affinities; coincident edges are not merged.
    /// Each cluster contributes its leading edge followed by its trailing edge.
    /// An empty line contributes a single default caret.
    pub fn visual_carets(&self, line_height: Pixels) -> Vec<(TextCaret, Point<Pixels>)> {
        let mut carets = Vec::new();
        for cluster in self.unwrapped_layout.clusters() {
            let probe = (cluster.left + cluster.right) / 2.;
            let row = self.wrap_boundaries.partition_point(|boundary| {
                self.runs()[boundary.run_ix].glyphs[boundary.glyph_ix]
                    .position
                    .x
                    <= probe
            });
            let start = self.row_extent(row).unwrap().start;
            carets.push((
                cluster.range.start.into(),
                point(cluster.leading() - start, line_height * row),
            ));
            carets.push((
                TextCaret {
                    index: cluster.range.end,
                    affinity: CaretAffinity::Upstream,
                },
                point(cluster.trailing() - start, line_height * row),
            ));
        }
        if carets.is_empty() {
            carets.push((TextCaret::default(), Point::default()));
        }
        carets
    }

    /// Selection rectangles in displayed coordinates, with a separate rectangle
    /// for each disjoint visual interval and wrapped row.
    pub fn selection_bounds(
        &self,
        selected: Range<usize>,
        line_height: Pixels,
    ) -> Vec<crate::Bounds<Pixels>> {
        let ranges = self.unwrapped_layout.selection_ranges(selected);
        let mut bounds = Vec::new();
        for row in 0..=self.wrap_boundaries.len() {
            let extent = self.row_extent(row).unwrap();
            for range in &ranges {
                let left = range.start.max(extent.start);
                let right = range.end.min(extent.end);
                if right > left {
                    bounds.push(crate::Bounds::new(
                        point(left - extent.start, line_height * row),
                        crate::size(right - left, line_height),
                    ));
                }
            }
        }
        bounds
    }
}

pub(crate) struct LineLayoutCache {
    previous_frame: Mutex<FrameCache>,
    current_frame: RwLock<FrameCache>,
    platform_text_system: Arc<dyn PlatformTextSystem>,
}

#[derive(Default)]
struct FrameCache {
    lines: FxHashMap<Arc<CacheKey>, Arc<LineLayout>>,
    wrapped_lines: FxHashMap<Arc<CacheKey>, Arc<WrappedLineLayout>>,
    used_lines: Vec<Arc<CacheKey>>,
    used_wrapped_lines: Vec<Arc<CacheKey>>,

    // Content-addressable caches keyed by caller-provided text hash + layout params.
    // These allow cache hits without materializing a contiguous `SharedString`.
    //
    // IMPORTANT: To support allocation-free lookups, we store these maps using a key type
    // (`HashedCacheKeyRef`) that can be computed without building a contiguous `&str`/`SharedString`.
    // On miss, we allocate once and store under an owned `HashedCacheKey`.
    lines_by_hash: FxHashMap<Arc<HashedCacheKey>, Arc<LineLayout>>,
    wrapped_lines_by_hash: FxHashMap<Arc<HashedCacheKey>, Arc<WrappedLineLayout>>,
    used_lines_by_hash: Vec<Arc<HashedCacheKey>>,
    used_wrapped_lines_by_hash: Vec<Arc<HashedCacheKey>>,
}

#[derive(Clone, Default)]
pub(crate) struct LineLayoutIndex {
    lines_index: usize,
    wrapped_lines_index: usize,
    lines_by_hash_index: usize,
    wrapped_lines_by_hash_index: usize,
}

impl LineLayoutCache {
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        Self {
            previous_frame: Mutex::default(),
            current_frame: RwLock::default(),
            platform_text_system,
        }
    }

    pub fn layout_index(&self) -> LineLayoutIndex {
        let frame = self.current_frame.read();
        LineLayoutIndex {
            lines_index: frame.used_lines.len(),
            wrapped_lines_index: frame.used_wrapped_lines.len(),
            lines_by_hash_index: frame.used_lines_by_hash.len(),
            wrapped_lines_by_hash_index: frame.used_wrapped_lines_by_hash.len(),
        }
    }

    pub fn reuse_layouts(&self, range: Range<LineLayoutIndex>) {
        let mut previous_frame = &mut *self.previous_frame.lock();
        let mut current_frame = &mut *self.current_frame.write();

        for key in &previous_frame.used_lines[range.start.lines_index..range.end.lines_index] {
            if let Some((key, line)) = previous_frame.lines.remove_entry(key) {
                current_frame.lines.insert(key, line);
            }
            current_frame.used_lines.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines
            [range.start.wrapped_lines_index..range.end.wrapped_lines_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines.remove_entry(key) {
                current_frame.wrapped_lines.insert(key, line);
            }
            current_frame.used_wrapped_lines.push(key.clone());
        }

        for key in &previous_frame.used_lines_by_hash
            [range.start.lines_by_hash_index..range.end.lines_by_hash_index]
        {
            if let Some((key, line)) = previous_frame.lines_by_hash.remove_entry(key) {
                current_frame.lines_by_hash.insert(key, line);
            }
            current_frame.used_lines_by_hash.push(key.clone());
        }

        for key in &previous_frame.used_wrapped_lines_by_hash
            [range.start.wrapped_lines_by_hash_index..range.end.wrapped_lines_by_hash_index]
        {
            if let Some((key, line)) = previous_frame.wrapped_lines_by_hash.remove_entry(key) {
                current_frame.wrapped_lines_by_hash.insert(key, line);
            }
            current_frame.used_wrapped_lines_by_hash.push(key.clone());
        }
    }

    pub fn truncate_layouts(&self, index: LineLayoutIndex) {
        let mut current_frame = &mut *self.current_frame.write();
        current_frame.used_lines.truncate(index.lines_index);
        current_frame
            .used_wrapped_lines
            .truncate(index.wrapped_lines_index);
        current_frame
            .used_lines_by_hash
            .truncate(index.lines_by_hash_index);
        current_frame
            .used_wrapped_lines_by_hash
            .truncate(index.wrapped_lines_by_hash_index);
    }

    pub fn finish_frame(&self) {
        let mut prev_frame = self.previous_frame.lock();
        let mut curr_frame = self.current_frame.write();
        std::mem::swap(&mut *prev_frame, &mut *curr_frame);
        curr_frame.lines.clear();
        curr_frame.wrapped_lines.clear();
        curr_frame.used_lines.clear();
        curr_frame.used_wrapped_lines.clear();

        curr_frame.lines_by_hash.clear();
        curr_frame.wrapped_lines_by_hash.clear();
        curr_frame.used_lines_by_hash.clear();
        curr_frame.used_wrapped_lines_by_hash.clear();
    }

    pub fn layout_wrapped_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        wrap_width: Option<Pixels>,
        max_lines: Option<usize>,
    ) -> Arc<WrappedLineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width,
            force_width: None,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.wrapped_lines.get(key) {
            return layout.clone();
        }

        let previous_frame_entry = self.previous_frame.lock().wrapped_lines.remove_entry(key);
        if let Some((key, layout)) = previous_frame_entry {
            let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);
            layout
        } else {
            drop(current_frame);
            let text = SharedString::from(text);
            let unwrapped_layout = self.layout_line::<&SharedString>(&text, font_size, runs, None);
            let wrap_boundaries = if let Some(wrap_width) = wrap_width {
                unwrapped_layout.compute_wrap_boundaries(text.as_ref(), wrap_width, max_lines)
            } else {
                SmallVec::new()
            };
            let layout = Arc::new(WrappedLineLayout {
                unwrapped_layout,
                wrap_boundaries,
                wrap_width,
            });
            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width,
                force_width: None,
            });

            let mut current_frame = self.current_frame.write();
            current_frame
                .wrapped_lines
                .insert(key.clone(), layout.clone());
            current_frame.used_wrapped_lines.push(key);

            layout
        }
    }

    pub fn layout_line<Text>(
        &self,
        text: Text,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
    ) -> Arc<LineLayout>
    where
        Text: AsRef<str>,
        SharedString: From<Text>,
    {
        let key = &CacheKeyRef {
            text: text.as_ref(),
            font_size,
            runs,
            wrap_width: None,
            force_width,
        } as &dyn AsCacheKeyRef;

        let current_frame = self.current_frame.upgradable_read();
        if let Some(layout) = current_frame.lines.get(key) {
            return layout.clone();
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);
        if let Some((key, layout)) = self.previous_frame.lock().lines.remove_entry(key) {
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        } else {
            let text = SharedString::from(text);
            let mut layout = self
                .platform_text_system
                .layout_line(&text, font_size, runs);

            if let Some(force_width) = force_width {
                apply_force_width_to_layout(&mut layout, force_width);
            }

            let key = Arc::new(CacheKey {
                text,
                font_size,
                runs: SmallVec::from(runs),
                wrap_width: None,
                force_width,
            });
            let layout = Arc::new(layout);
            current_frame.lines.insert(key.clone(), layout.clone());
            current_frame.used_lines.push(key);
            layout
        }
    }

    /// Try to retrieve a previously-shaped line layout using a caller-provided content hash.
    ///
    /// This is a *non-allocating* cache probe: it does not materialize any text. If the layout
    /// is not already cached in either the current frame or previous frame, returns `None`.
    ///
    /// Contract (caller enforced):
    /// - Same `text_hash` implies identical text content (collision risk accepted by caller).
    /// - `text_len` should be the UTF-8 byte length of the text (helps reduce accidental collisions).
    pub fn try_layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
    ) -> Option<Arc<LineLayout>> {
        let key_ref = HashedCacheKeyRef {
            text_hash,
            text_len,
            font_size,
            runs,
            wrap_width: None,
            force_width,
        };

        let current_frame = self.current_frame.read();
        if let Some((_, layout)) = current_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return Some(layout.clone());
        }

        let previous_frame = self.previous_frame.lock();
        if let Some((_, layout)) = previous_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return Some(layout.clone());
        }

        None
    }

    /// Layout a line of text using a caller-provided content hash as the cache key.
    ///
    /// This enables cache hits without materializing a contiguous `SharedString` for `text`.
    /// If the cache misses, `materialize_text` is invoked to produce the `SharedString` for shaping.
    ///
    /// Contract (caller enforced):
    /// - Same `text_hash` implies identical text content (collision risk accepted by caller).
    /// - `text_len` should be the UTF-8 byte length of the text (helps reduce accidental collisions).
    pub fn layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[FontRun],
        force_width: Option<Pixels>,
        materialize_text: impl FnOnce() -> SharedString,
    ) -> Arc<LineLayout> {
        let key_ref = HashedCacheKeyRef {
            text_hash,
            text_len,
            font_size,
            runs,
            wrap_width: None,
            force_width,
        };

        // Fast path: already cached (no allocation).
        let current_frame = self.current_frame.upgradable_read();
        if let Some((_, layout)) = current_frame.lines_by_hash.iter().find(|(key, _)| {
            HashedCacheKeyRef {
                text_hash: key.text_hash,
                text_len: key.text_len,
                font_size: key.font_size,
                runs: key.runs.as_slice(),
                wrap_width: key.wrap_width,
                force_width: key.force_width,
            } == key_ref
        }) {
            return layout.clone();
        }

        let mut current_frame = RwLockUpgradableReadGuard::upgrade(current_frame);

        // Try to reuse from previous frame without allocating; do a linear scan to find a matching key.
        // (We avoid `drain()` here because it would eagerly move all entries.)
        let mut previous_frame = self.previous_frame.lock();
        if let Some(existing_key) = previous_frame
            .used_lines_by_hash
            .iter()
            .find(|key| {
                HashedCacheKeyRef {
                    text_hash: key.text_hash,
                    text_len: key.text_len,
                    font_size: key.font_size,
                    runs: key.runs.as_slice(),
                    wrap_width: key.wrap_width,
                    force_width: key.force_width,
                } == key_ref
            })
            .cloned()
        {
            if let Some((key, layout)) = previous_frame.lines_by_hash.remove_entry(&existing_key) {
                current_frame
                    .lines_by_hash
                    .insert(key.clone(), layout.clone());
                current_frame.used_lines_by_hash.push(key);
                return layout;
            }
        }

        let text = materialize_text();
        let mut layout = self
            .platform_text_system
            .layout_line(&text, font_size, runs);

        if let Some(force_width) = force_width {
            apply_force_width_to_layout(&mut layout, force_width);
        }

        let key = Arc::new(HashedCacheKey {
            text_hash,
            text_len,
            font_size,
            runs: SmallVec::from(runs),
            wrap_width: None,
            force_width,
        });
        let layout = Arc::new(layout);
        current_frame
            .lines_by_hash
            .insert(key.clone(), layout.clone());
        current_frame.used_lines_by_hash.push(key);
        layout
    }
}

// Combining marks (e.g. Thai vowel signs, Arabic diacritics) are shaped by
// HarfBuzz at the same x position as their base character. The force-width
// loop must not advance the cell counter for these zero-advance glyphs,
// otherwise they get displaced into the next cell. We detect them by checking
// whether shaped x has advanced by at least half a cell beyond the last base.
fn apply_force_width_to_layout(layout: &mut LineLayout, force_width: Pixels) {
    let mut glyph_pos: usize = 0;
    // NEG_INFINITY ensures the first glyph is always classified as a base.
    let mut last_base_shaped_x = px(f32::NEG_INFINITY);
    let mut last_base_actual_x = px(0.);

    for run in layout.runs.iter_mut() {
        for glyph in run.glyphs.iter_mut() {
            let shaped_x = glyph.position.x;

            if shaped_x > last_base_shaped_x + force_width * 0.5 {
                let forced_x = glyph_pos * force_width;
                if (shaped_x - forced_x).abs() > px(1.) {
                    glyph.position.x = forced_x;
                }
                last_base_shaped_x = shaped_x;
                last_base_actual_x = glyph.position.x;
                if glyph.advance > px(0.) {
                    glyph.advance = force_width;
                }
                glyph_pos += 1;
            } else {
                glyph.position.x = last_base_actual_x + (shaped_x - last_base_shaped_x);
            }
        }
    }
}

/// A run of text with a single font.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[expect(missing_docs)]
pub struct FontRun {
    pub len: usize,
    pub font_id: FontId,
}

trait AsCacheKeyRef {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_>;
}

#[derive(Clone, Debug, Eq)]
struct CacheKey {
    text: SharedString,
    font_size: Pixels,
    runs: SmallVec<[FontRun; 1]>,
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct CacheKeyRef<'a> {
    text: &'a str,
    font_size: Pixels,
    runs: &'a [FontRun],
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Clone, Debug)]
struct HashedCacheKey {
    text_hash: u64,
    text_len: usize,
    font_size: Pixels,
    runs: SmallVec<[FontRun; 1]>,
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

#[derive(Copy, Clone)]
struct HashedCacheKeyRef<'a> {
    text_hash: u64,
    text_len: usize,
    font_size: Pixels,
    runs: &'a [FontRun],
    wrap_width: Option<Pixels>,
    force_width: Option<Pixels>,
}

impl PartialEq for dyn AsCacheKeyRef + '_ {
    fn eq(&self, other: &dyn AsCacheKeyRef) -> bool {
        self.as_cache_key_ref() == other.as_cache_key_ref()
    }
}

impl PartialEq for HashedCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.text_len == other.text_len
            && self.font_size == other.font_size
            && self.runs.as_slice() == other.runs.as_slice()
            && self.wrap_width == other.wrap_width
            && self.force_width == other.force_width
    }
}

impl Eq for HashedCacheKey {}

impl Hash for HashedCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text_hash.hash(state);
        self.text_len.hash(state);
        self.font_size.hash(state);
        self.runs.as_slice().hash(state);
        self.wrap_width.hash(state);
        self.force_width.hash(state);
    }
}

impl PartialEq for HashedCacheKeyRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.text_len == other.text_len
            && self.font_size == other.font_size
            && self.runs == other.runs
            && self.wrap_width == other.wrap_width
            && self.force_width == other.force_width
    }
}

impl Eq for HashedCacheKeyRef<'_> {}

impl Hash for HashedCacheKeyRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.text_hash.hash(state);
        self.text_len.hash(state);
        self.font_size.hash(state);
        self.runs.hash(state);
        self.wrap_width.hash(state);
        self.force_width.hash(state);
    }
}

impl Eq for dyn AsCacheKeyRef + '_ {}

impl Hash for dyn AsCacheKeyRef + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state)
    }
}

impl AsCacheKeyRef for CacheKey {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        CacheKeyRef {
            text: &self.text,
            font_size: self.font_size,
            runs: self.runs.as_slice(),
            wrap_width: self.wrap_width,
            force_width: self.force_width,
        }
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_cache_key_ref().eq(&other.as_cache_key_ref())
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_cache_key_ref().hash(state);
    }
}

impl<'a> Borrow<dyn AsCacheKeyRef + 'a> for Arc<CacheKey> {
    fn borrow(&self) -> &(dyn AsCacheKeyRef + 'a) {
        self.as_ref() as &dyn AsCacheKeyRef
    }
}

impl AsCacheKeyRef for CacheKeyRef<'_> {
    fn as_cache_key_ref(&self) -> CacheKeyRef<'_> {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GlyphId;

    fn bidi_layout() -> LineLayout {
        // Logical text: "aאב12". Visual order: a, 1, 2, ב, א.
        let mut layout = make_layout(
            [
                (0, 1, false),
                (5, 6, false),
                (6, 7, false),
                (3, 5, true),
                (1, 3, true),
            ]
            .into_iter()
            .enumerate()
            .map(|(visual, (index, end, rtl))| ShapedGlyph {
                id: GlyphId(0),
                position: point(px(visual as f32 * 10.), px(0.)),
                index,
                cluster_end: end,
                advance: px(10.),
                is_rtl: rtl,
                is_emoji: false,
            })
            .collect(),
        );
        layout.len = 7;
        layout.width = px(50.);
        layout
    }

    #[test]
    fn bidi_caret_uses_logical_cluster_edges() {
        let layout = bidi_layout();
        assert_eq!(layout.x_for_index(1), px(50.));
        assert_eq!(layout.x_for_index(3), px(40.));
        assert_eq!(layout.x_for_index(5), px(10.));
        assert_eq!(layout.x_for_index(7), px(30.));
        assert_eq!(layout.closest_index_for_x(px(48.)), 1);
        assert_eq!(layout.closest_index_for_x(px(42.)), 3);
        assert_eq!(layout.closest_index_for_x(px(32.)), 5);
        assert_eq!(layout.closest_index_for_x(px(60.)), 1);
    }

    #[test]
    fn bidi_selection_preserves_disjoint_visual_spans() {
        let layout = bidi_layout();
        assert_eq!(
            layout.selection_ranges(0..3),
            vec![px(0.)..px(10.), px(40.)..px(50.)]
        );
        assert_eq!(layout.selection_ranges(1..5), vec![px(30.)..px(50.)]);
        assert_eq!(layout.selection_ranges(0..7), vec![px(0.)..px(50.)]);
        assert!(layout.selection_ranges(3..3).is_empty());
    }

    #[test]
    fn bidi_caret_hit_testing_preserves_both_sides_of_a_boundary() {
        let layout = bidi_layout();
        let left = layout.caret_for_x(px(9.));
        let right = layout.caret_for_x(px(49.));
        assert_eq!(left.index, 1);
        assert_eq!(right.index, 1);
        assert_eq!(left.affinity, CaretAffinity::Upstream);
        assert_eq!(right.affinity, CaretAffinity::Downstream);
        assert_eq!(layout.x_for_caret(left), px(10.));
        assert_eq!(layout.x_for_caret(right), px(50.));
    }

    #[test]
    fn bidi_wrapped_geometry_matches_displayed_rows() {
        let wrapped = WrappedLineLayout {
            unwrapped_layout: Arc::new(bidi_layout()),
            wrap_boundaries: smallvec::smallvec![WrapBoundary {
                run_ix: 0,
                glyph_ix: 3
            }],
            wrap_width: Some(px(30.)),
        };
        assert_eq!(
            wrapped.position_for_index(1, px(20.)),
            Some(point(px(20.), px(20.)))
        );
        assert_eq!(
            wrapped.position_for_index(3, px(20.)),
            Some(point(px(10.), px(20.)))
        );
        assert_eq!(
            wrapped.closest_index_for_position(point(px(19.), px(25.)), px(20.)),
            Ok(1)
        );
        assert_eq!(
            wrapped.closest_index_for_position(point(px(-5.), px(25.)), px(20.)),
            Err(5)
        );
        assert_eq!(
            wrapped.selection_bounds(0..3, px(20.)),
            vec![
                crate::Bounds::new(point(px(0.), px(0.)), crate::size(px(10.), px(20.))),
                crate::Bounds::new(point(px(10.), px(20.)), crate::size(px(10.), px(20.))),
            ]
        );
    }

    #[test]
    fn rtl_combining_cluster_has_one_hitbox_and_utf8_boundaries() {
        let mut layout = make_layout(vec![
            ShapedGlyph {
                index: 4,
                cluster_end: 6,
                advance: px(10.),
                is_rtl: true,
                ..glyph_at(0., 4)
            },
            ShapedGlyph {
                index: 0,
                cluster_end: 4,
                advance: px(10.),
                is_rtl: true,
                ..glyph_at(10., 0)
            },
            ShapedGlyph {
                index: 0,
                cluster_end: 4,
                advance: px(0.),
                is_rtl: true,
                ..glyph_at(9., 0)
            },
        ]);
        layout.len = 6;
        layout.width = px(20.);
        assert_eq!(layout.closest_index_for_x(px(-5.)), 6);
        assert_eq!(layout.closest_index_for_x(px(25.)), 0);
        assert_eq!(layout.selection_ranges(0..2), vec![px(10.)..px(20.)]);
        let wrapped = WrappedLineLayout {
            unwrapped_layout: Arc::new(layout),
            ..Default::default()
        };
        assert_eq!(
            wrapped.position_for_index(0, px(20.)),
            Some(point(px(20.), px(0.)))
        );
        assert_eq!(
            wrapped.position_for_index(6, px(20.)),
            Some(point(px(0.), px(0.)))
        );
    }

    fn glyph_at(x: f32, index: usize) -> ShapedGlyph {
        ShapedGlyph {
            id: GlyphId(0),
            position: point(px(x), px(0.)),
            index,
            cluster_end: index + 1,
            advance: px(8.),
            is_rtl: false,
            is_emoji: false,
        }
    }

    fn make_layout(glyphs: Vec<ShapedGlyph>) -> LineLayout {
        LineLayout {
            font_size: px(16.),
            width: px(100.),
            ascent: px(12.),
            descent: px(4.),
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs,
            }],
            len: 0,
        }
    }

    fn glyph_x_positions(layout: &LineLayout) -> Vec<f32> {
        layout.runs[0]
            .glyphs
            .iter()
            .map(|g| f32::from(g.position.x))
            .collect()
    }

    #[test]
    fn test_force_width_latin_unchanged() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(8., 1), glyph_at(16., 2)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_marks_not_advanced() {
        let cell_width = px(8.);
        // Simulates Thai "กี" — base consonant at x=0, combining vowel also at x=0
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // ี (combining mark, same x)
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0.]);
    }

    #[test]
    fn test_force_width_base_after_combining_mark() {
        let cell_width = px(8.);
        let mut layout = make_layout(vec![glyph_at(0., 0), glyph_at(0., 3), glyph_at(8., 6)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 8.]);
    }

    #[test]
    fn test_force_width_multiple_combining_marks() {
        let cell_width = px(8.);
        // Simulates "ก้" — base + vowel + tone mark (two combining marks stacked)
        let mut layout = make_layout(vec![
            glyph_at(0., 0), // ก (base)
            glyph_at(0., 3), // vowel (combining)
            glyph_at(0., 6), // tone mark (combining)
            glyph_at(8., 9), // next base
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0., 0., 0., 8.]);
    }

    #[test]
    fn test_force_width_corrects_drifted_base_positions() {
        let cell_width = px(8.);
        // Font metrics don't perfectly match cell grid — glyphs drift >1px from cell boundary
        let mut layout = make_layout(vec![
            glyph_at(0.5, 0),  // within 1px tolerance, kept as-is
            glyph_at(10.2, 1), // >1px off from 8.0, corrected
            glyph_at(19.8, 2), // >1px off from 16.0, corrected
        ]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 8., 16.]);
    }

    #[test]
    fn test_force_width_combining_mark_after_within_tolerance_base() {
        let cell_width = px(8.);
        // Base glyph is within 1px of grid so it keeps its shaped position.
        // The combining mark must align to the base's actual position, not the grid slot.
        let mut layout = make_layout(vec![glyph_at(0.5, 0), glyph_at(0.5, 3)]);

        apply_force_width_to_layout(&mut layout, cell_width);

        let positions = glyph_x_positions(&layout);
        assert_eq!(positions, vec![0.5, 0.5]);
    }
}
