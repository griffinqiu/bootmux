//! A tmux-independent binary space partitioning (BSP) layout model.
//!
//! A [`Layout::Split`] always describes the position of its `second` child
//! relative to its `first` child:
//!
//! - [`SplitDirection::Right`] puts `second` to the right of `first`.
//! - [`SplitDirection::Down`] puts `second` below `first`.
//!
//! `ratio` is the share assigned to the `first` child and must be strictly
//! between zero and one.  The model deliberately contains no terminal
//! dimensions or tmux commands, so callers can render it for any backend.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// The smallest share accepted by [`PaneChainBuilder`].
pub const MIN_PANE_SHARE: f64 = 0.1;

/// The largest share accepted by [`PaneChainBuilder`].
pub const MAX_PANE_SHARE: f64 = 0.9;

/// Where the second child of a split is placed relative to the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SplitDirection {
    /// Place the second child to the right of the first.
    Right,
    /// Place the second child below the first.
    Down,
}

impl SplitDirection {
    /// Returns the stable, human-readable name of this direction.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

impl fmt::Display for SplitDirection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A shorter alias useful to layout renderers.
pub type Direction = SplitDirection;

/// A neutral binary space partitioning layout.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// A leaf containing the zero-based pane index.
    Pane(usize),
    /// A binary split.
    Split {
        /// Where `second` is placed relative to `first`.
        direction: SplitDirection,
        /// The share assigned to `first`.
        ratio: f64,
        /// The left or upper child.
        first: Box<Layout>,
        /// The right or lower child.
        second: Box<Layout>,
    },
}

/// An alias that emphasizes that layouts are trees of nodes.
pub type LayoutNode = Layout;

impl Layout {
    /// Creates a pane leaf.
    pub const fn pane(index: usize) -> Self {
        Self::Pane(index)
    }

    /// Creates and validates a split.
    ///
    /// Besides checking `ratio`, this rejects duplicate pane indices across
    /// the two child trees.
    pub fn split(
        direction: SplitDirection,
        ratio: f64,
        first: Layout,
        second: Layout,
    ) -> Result<Self, LayoutError> {
        validate_ratio(ratio)?;
        let layout = Self::split_known_valid(direction, ratio, first, second);
        layout.validate()?;
        Ok(layout)
    }

    /// Builds the deterministic default tiled layout.
    ///
    /// The number of columns is `ceil(sqrt(pane_count))`. Panes are assigned
    /// to `ceil(pane_count / columns)` rows in index order, with row sizes
    /// differing by at most one. Rows receive equal heights, and panes within
    /// each row receive equal widths.
    pub fn default_tiled(pane_count: usize) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;

        let columns = ceil_sqrt(pane_count);
        let row_count = div_ceil(pane_count, columns);
        let short_row_len = pane_count / row_count;
        let longer_rows = pane_count % row_count;

        let mut next_pane = 0;
        let mut rows = Vec::with_capacity(row_count);
        for row_index in 0..row_count {
            let row_len = short_row_len + usize::from(row_index < longer_rows);
            let row = equal_panes(next_pane..next_pane + row_len, SplitDirection::Right);
            next_pane += row_len;
            rows.push(row);
        }

        Ok(equal_layouts(rows, SplitDirection::Down))
    }

    /// Alias for [`Layout::default_tiled`].
    pub fn tiled(pane_count: usize) -> Result<Self, LayoutError> {
        Self::default_tiled(pane_count)
    }

    /// Places every pane from left to right with an equal share.
    pub fn even_horizontal(pane_count: usize) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;
        Ok(equal_panes(0..pane_count, SplitDirection::Right))
    }

    /// Places every pane from top to bottom with an equal share.
    pub fn even_vertical(pane_count: usize) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;
        Ok(equal_panes(0..pane_count, SplitDirection::Down))
    }

    /// Places pane zero above the other panes.
    ///
    /// Pane zero receives half the height. The remaining panes share the
    /// bottom half equally from left to right.
    pub fn main_horizontal(pane_count: usize) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;
        if pane_count == 1 {
            return Ok(Self::Pane(0));
        }

        let remaining = equal_panes(1..pane_count, SplitDirection::Right);
        Ok(Self::split_known_valid(
            SplitDirection::Down,
            0.5,
            Self::Pane(0),
            remaining,
        ))
    }

    /// Places pane zero to the left of the other panes.
    ///
    /// Pane zero receives half the width. The remaining panes share the right
    /// half equally from top to bottom.
    pub fn main_vertical(pane_count: usize) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;
        if pane_count == 1 {
            return Ok(Self::Pane(0));
        }

        let remaining = equal_panes(1..pane_count, SplitDirection::Down);
        Ok(Self::split_known_valid(
            SplitDirection::Right,
            0.5,
            Self::Pane(0),
            remaining,
        ))
    }

    /// Builds a sequential pane chain with one direction and ratio.
    ///
    /// Pane zero is the initial leaf. Each subsequent pane splits the leaf
    /// created immediately before it. `existing_pane_share` is therefore the
    /// share assigned to that previous leaf at every split.
    pub fn pane_chain(
        pane_count: usize,
        direction: SplitDirection,
        existing_pane_share: f64,
    ) -> Result<Self, LayoutError> {
        require_panes(pane_count)?;
        validate_pane_share(existing_pane_share)?;

        let mut builder = PaneChainBuilder::new(0);
        for _ in 1..pane_count {
            builder = builder.split(direction, existing_pane_share)?;
        }
        Ok(builder.build())
    }

    /// Parses a checksum-prefixed tmux serialized layout.
    pub fn parse_tmux(serialized: &str) -> Result<Self, TmuxLayoutParseError> {
        parse_tmux_layout(serialized)
    }

    /// Returns pane indices in visual, depth-first order.
    pub fn pane_indices(&self) -> Vec<usize> {
        let mut panes = Vec::with_capacity(self.pane_count());
        self.collect_panes(&mut panes);
        panes
    }

    /// Returns the number of pane leaves in this layout.
    pub fn pane_count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    /// Returns whether this layout contains `pane_index`.
    pub fn contains_pane(&self, pane_index: usize) -> bool {
        match self {
            Self::Pane(index) => *index == pane_index,
            Self::Split { first, second, .. } => {
                first.contains_pane(pane_index) || second.contains_pane(pane_index)
            }
        }
    }

    /// Validates ratios and verifies that every pane index is unique.
    ///
    /// This is useful after directly constructing public enum variants.
    pub fn validate(&self) -> Result<(), LayoutError> {
        fn visit(layout: &Layout, seen: &mut BTreeSet<usize>) -> Result<(), LayoutError> {
            match layout {
                Layout::Pane(index) => {
                    if !seen.insert(*index) {
                        return Err(LayoutError::DuplicatePane { index: *index });
                    }
                }
                Layout::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    validate_ratio(*ratio)?;
                    visit(first, seen)?;
                    visit(second, seen)?;
                }
            }
            Ok(())
        }

        visit(self, &mut BTreeSet::new())
    }

    fn split_known_valid(
        direction: SplitDirection,
        ratio: f64,
        first: Layout,
        second: Layout,
    ) -> Self {
        Self::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    fn collect_panes(&self, panes: &mut Vec<usize>) {
        match self {
            Self::Pane(index) => panes.push(*index),
            Self::Split { first, second, .. } => {
                first.collect_panes(panes);
                second.collect_panes(panes);
            }
        }
    }
}

impl FromStr for Layout {
    type Err = TmuxLayoutParseError;

    fn from_str(serialized: &str) -> Result<Self, Self::Err> {
        Self::parse_tmux(serialized)
    }
}

/// Named layouts supported by the neutral model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayoutPreset {
    Tiled,
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
}

impl LayoutPreset {
    /// Builds this preset for panes numbered from zero to `pane_count - 1`.
    pub fn build(self, pane_count: usize) -> Result<Layout, LayoutError> {
        match self {
            Self::Tiled => Layout::default_tiled(pane_count),
            Self::EvenHorizontal => Layout::even_horizontal(pane_count),
            Self::EvenVertical => Layout::even_vertical(pane_count),
            Self::MainHorizontal => Layout::main_horizontal(pane_count),
            Self::MainVertical => Layout::main_vertical(pane_count),
        }
    }

    /// Returns the tmux-compatible spelling of this preset.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tiled => "tiled",
            Self::EvenHorizontal => "even-horizontal",
            Self::EvenVertical => "even-vertical",
            Self::MainHorizontal => "main-horizontal",
            Self::MainVertical => "main-vertical",
        }
    }
}

impl fmt::Display for LayoutPreset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for LayoutPreset {
    type Err = UnknownLayoutPreset;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "tiled" => Ok(Self::Tiled),
            "even-horizontal" => Ok(Self::EvenHorizontal),
            "even-vertical" => Ok(Self::EvenVertical),
            "main-horizontal" => Ok(Self::MainHorizontal),
            "main-vertical" => Ok(Self::MainVertical),
            _ => Err(UnknownLayoutPreset(value.to_owned())),
        }
    }
}

/// Error returned when parsing an unknown named layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownLayoutPreset(pub String);

impl fmt::Display for UnknownLayoutPreset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown layout preset {:?}", self.0)
    }
}

impl Error for UnknownLayoutPreset {}

/// Errors raised while constructing or validating a neutral layout.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    /// A layout must contain at least one pane.
    Empty,
    /// A general BSP split ratio was not finite and strictly between 0 and 1.
    InvalidRatio { ratio: f64 },
    /// A pane-chain share was outside its intentionally conservative range.
    InvalidPaneShare { ratio: f64 },
    /// The same pane index appeared more than once.
    DuplicatePane { index: usize },
    /// No sequential pane index remains after `usize::MAX`.
    PaneIndexOverflow,
    /// The pane-chain tail was unexpectedly absent.
    PaneNotFound { index: usize },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a layout requires at least one pane"),
            Self::InvalidRatio { ratio } => {
                write!(
                    formatter,
                    "split ratio must be finite and strictly between 0 and 1, got {ratio}"
                )
            }
            Self::InvalidPaneShare { ratio } => {
                write!(
                    formatter,
                    "existing-pane share must be between {MIN_PANE_SHARE} and \
                     {MAX_PANE_SHARE}, got {ratio}"
                )
            }
            Self::DuplicatePane { index } => {
                write!(formatter, "pane index {index} occurs more than once")
            }
            Self::PaneIndexOverflow => {
                formatter.write_str("cannot allocate another sequential pane index")
            }
            Self::PaneNotFound { index } => {
                write!(formatter, "pane index {index} was not found in the layout")
            }
        }
    }
}

impl Error for LayoutError {}

/// Builds a chain by repeatedly splitting the leaf created most recently.
///
/// The builder starts with one pane. [`PaneChainBuilder::split`] allocates
/// sequential pane indices, while [`PaneChainBuilder::split_pane`] lets the
/// caller choose an index explicitly.
#[derive(Debug, Clone)]
pub struct PaneChainBuilder {
    layout: Layout,
    tail_pane: usize,
    next_pane: Option<usize>,
}

impl PaneChainBuilder {
    /// Starts a chain at `first_pane`.
    pub fn new(first_pane: usize) -> Self {
        Self {
            layout: Layout::Pane(first_pane),
            tail_pane: first_pane,
            next_pane: first_pane.checked_add(1),
        }
    }

    /// Splits the most recently created pane and allocates the next index.
    pub fn split(
        self,
        direction: SplitDirection,
        existing_pane_share: f64,
    ) -> Result<Self, LayoutError> {
        let pane_index = self.next_pane.ok_or(LayoutError::PaneIndexOverflow)?;
        self.split_pane(pane_index, direction, existing_pane_share)
    }

    /// Splits the most recently created pane, placing `pane_index` second.
    pub fn split_pane(
        mut self,
        pane_index: usize,
        direction: SplitDirection,
        existing_pane_share: f64,
    ) -> Result<Self, LayoutError> {
        validate_pane_share(existing_pane_share)?;
        if self.layout.contains_pane(pane_index) {
            return Err(LayoutError::DuplicatePane { index: pane_index });
        }

        let previous_tail = self.tail_pane;
        let replacement = Layout::split_known_valid(
            direction,
            existing_pane_share,
            Layout::Pane(previous_tail),
            Layout::Pane(pane_index),
        );
        self.layout = replace_pane(self.layout, previous_tail, replacement)?;
        self.tail_pane = pane_index;

        let largest_index = self
            .layout
            .pane_indices()
            .into_iter()
            .max()
            .expect("a pane chain always contains a pane");
        self.next_pane = largest_index.checked_add(1);
        Ok(self)
    }

    /// Convenience wrapper for a right split.
    pub fn split_right(self, existing_pane_share: f64) -> Result<Self, LayoutError> {
        self.split(SplitDirection::Right, existing_pane_share)
    }

    /// Convenience wrapper for a downward split.
    pub fn split_down(self, existing_pane_share: f64) -> Result<Self, LayoutError> {
        self.split(SplitDirection::Down, existing_pane_share)
    }

    /// Borrows the layout built so far.
    pub const fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Returns the pane that the next call to `split` will replace.
    pub const fn tail_pane(&self) -> usize {
        self.tail_pane
    }

    /// Finishes the chain.
    pub fn build(self) -> Layout {
        self.layout
    }
}

fn replace_pane(
    layout: Layout,
    pane_index: usize,
    replacement: Layout,
) -> Result<Layout, LayoutError> {
    match layout {
        Layout::Pane(index) if index == pane_index => Ok(replacement),
        Layout::Pane(_) => Err(LayoutError::PaneNotFound { index: pane_index }),
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            if first.contains_pane(pane_index) {
                Ok(Layout::split_known_valid(
                    direction,
                    ratio,
                    replace_pane(*first, pane_index, replacement)?,
                    *second,
                ))
            } else if second.contains_pane(pane_index) {
                Ok(Layout::split_known_valid(
                    direction,
                    ratio,
                    *first,
                    replace_pane(*second, pane_index, replacement)?,
                ))
            } else {
                Err(LayoutError::PaneNotFound { index: pane_index })
            }
        }
    }
}

fn validate_ratio(ratio: f64) -> Result<(), LayoutError> {
    if ratio.is_finite() && ratio > 0.0 && ratio < 1.0 {
        Ok(())
    } else {
        Err(LayoutError::InvalidRatio { ratio })
    }
}

fn validate_pane_share(ratio: f64) -> Result<(), LayoutError> {
    if ratio.is_finite() && (MIN_PANE_SHARE..=MAX_PANE_SHARE).contains(&ratio) {
        Ok(())
    } else {
        Err(LayoutError::InvalidPaneShare { ratio })
    }
}

fn require_panes(pane_count: usize) -> Result<(), LayoutError> {
    if pane_count == 0 {
        Err(LayoutError::Empty)
    } else {
        Ok(())
    }
}

fn equal_panes(indices: std::ops::Range<usize>, direction: SplitDirection) -> Layout {
    let layouts = indices.map(Layout::Pane).collect();
    equal_layouts(layouts, direction)
}

fn equal_layouts(layouts: Vec<Layout>, direction: SplitDirection) -> Layout {
    debug_assert!(!layouts.is_empty());

    fn build(mut layouts: Vec<Layout>, direction: SplitDirection) -> Layout {
        if layouts.len() == 1 {
            return layouts.pop().expect("the remaining layout count is exact");
        }

        let count = layouts.len();
        let first_count = count / 2;
        let second_layouts = layouts.split_off(first_count);
        let first = build(layouts, direction);
        let second = build(second_layouts, direction);
        Layout::split_known_valid(direction, first_count as f64 / count as f64, first, second)
    }

    build(layouts, direction)
}

fn div_ceil(value: usize, divisor: usize) -> usize {
    value.div_ceil(divisor)
}

fn ceil_sqrt(value: usize) -> usize {
    debug_assert!(value > 0);

    // Find the smallest x for which x >= ceil(value / x), avoiding x*x and
    // therefore avoiding overflow even for usize::MAX.
    let mut low = 1;
    let mut high = value;
    while low < high {
        let midpoint = low + (high - low) / 2;
        if midpoint >= div_ceil(value, midpoint) {
            high = midpoint;
        } else {
            low = midpoint + 1;
        }
    }
    low
}

/// Computes the checksum used at the start of a tmux serialized layout.
///
/// The checksum is computed over the payload after the first comma.
pub fn tmux_layout_checksum(payload: &str) -> u16 {
    payload.bytes().fold(0_u16, |checksum, byte| {
        let rotated = checksum.rotate_right(1);
        rotated.wrapping_add(u16::from(byte))
    })
}

/// Parses and validates a checksum-prefixed tmux serialized layout.
///
/// The accepted grammar is the one emitted by modern tmux:
///
/// ```text
/// checksum,widthxheight,x,y,pane
/// checksum,widthxheight,x,y{child,child,...}
/// checksum,widthxheight,x,y[child,child,...]
/// ```
///
/// `{...}` is a left-to-right (`Right`) container and `[...]` is a
/// top-to-bottom (`Down`) container. tmux may serialize more than two direct
/// children; those are deterministically converted into a right-associated
/// BSP tree. Dimensions, offsets, dividers, pane uniqueness, complete input
/// consumption, and checksum are all validated.
pub fn parse_tmux_layout(serialized: &str) -> Result<Layout, TmuxLayoutParseError> {
    let bytes = serialized.as_bytes();
    if bytes.len() < 5 {
        return Err(TmuxLayoutParseError::new(
            0,
            "expected four hexadecimal checksum digits followed by a comma",
        ));
    }
    if bytes[4] != b',' || !bytes[..4].iter().all(u8::is_ascii_hexdigit) {
        return Err(TmuxLayoutParseError::new(
            0,
            "expected four hexadecimal checksum digits followed by a comma",
        ));
    }

    let checksum_text =
        std::str::from_utf8(&bytes[..4]).expect("ASCII hexadecimal checksum bytes are valid UTF-8");
    let declared_checksum =
        u16::from_str_radix(checksum_text, 16).expect("four hexadecimal digits always fit in u16");
    let payload = &serialized[5..];
    let calculated_checksum = tmux_layout_checksum(payload);
    if declared_checksum != calculated_checksum {
        return Err(TmuxLayoutParseError::new(
            0,
            format!(
                "checksum mismatch: declared {declared_checksum:04x}, \
                 calculated {calculated_checksum:04x}"
            ),
        ));
    }

    let mut parser = TmuxParser::new(payload);
    let parsed = parser.parse_node()?;
    if !parser.is_finished() {
        return Err(parser.error("unexpected trailing input"));
    }

    let layout = parsed.into_layout();
    layout
        .validate()
        .map_err(|error| TmuxLayoutParseError::new(5, error.to_string()))?;
    Ok(layout)
}

/// An error in a checksum-prefixed tmux layout string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxLayoutParseError {
    position: usize,
    message: String,
}

impl TmuxLayoutParseError {
    fn new(position: usize, message: impl Into<String>) -> Self {
        Self {
            position,
            message: message.into(),
        }
    }

    /// Byte position in the complete serialized string.
    pub const fn position(&self) -> usize {
        self.position
    }

    /// A concise description of the parse or validation failure.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TmuxLayoutParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid tmux layout at byte {}: {}",
            self.position, self.message
        )
    }
}

impl Error for TmuxLayoutParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rectangle {
    width: usize,
    height: usize,
    x: usize,
    y: usize,
}

impl Rectangle {
    fn axis_length(self, direction: SplitDirection) -> usize {
        match direction {
            SplitDirection::Right => self.width,
            SplitDirection::Down => self.height,
        }
    }

    fn combine(first: Self, second: Self, direction: SplitDirection) -> Result<Self, &'static str> {
        match direction {
            SplitDirection::Right => {
                if first.y != second.y || first.height != second.height {
                    return Err("right-split children must have equal y offsets and heights");
                }
                let expected_x = first
                    .x
                    .checked_add(first.width)
                    .and_then(|edge| edge.checked_add(1))
                    .ok_or("right-split geometry overflows usize")?;
                if second.x != expected_x {
                    return Err("right-split children must be adjacent with one divider column");
                }
                let width = first
                    .width
                    .checked_add(1)
                    .and_then(|span| span.checked_add(second.width))
                    .ok_or("right-split geometry overflows usize")?;
                Ok(Self {
                    width,
                    height: first.height,
                    x: first.x,
                    y: first.y,
                })
            }
            SplitDirection::Down => {
                if first.x != second.x || first.width != second.width {
                    return Err("down-split children must have equal x offsets and widths");
                }
                let expected_y = first
                    .y
                    .checked_add(first.height)
                    .and_then(|edge| edge.checked_add(1))
                    .ok_or("down-split geometry overflows usize")?;
                if second.y != expected_y {
                    return Err("down-split children must be adjacent with one divider row");
                }
                let height = first
                    .height
                    .checked_add(1)
                    .and_then(|span| span.checked_add(second.height))
                    .ok_or("down-split geometry overflows usize")?;
                Ok(Self {
                    width: first.width,
                    height,
                    x: first.x,
                    y: first.y,
                })
            }
        }
    }
}

#[derive(Debug)]
struct ParsedNode {
    rectangle: Rectangle,
    kind: ParsedKind,
}

#[derive(Debug)]
enum ParsedKind {
    Pane(usize),
    Children {
        direction: SplitDirection,
        children: Vec<ParsedNode>,
    },
}

impl ParsedNode {
    fn into_layout(self) -> Layout {
        match self.kind {
            ParsedKind::Pane(index) => Layout::Pane(index),
            ParsedKind::Children {
                direction,
                children,
            } => {
                fn fold(
                    children: &mut std::vec::IntoIter<ParsedNode>,
                    remaining: usize,
                    direction: SplitDirection,
                ) -> (Layout, Rectangle) {
                    let first = children
                        .next()
                        .expect("validated tmux containers have enough children");
                    let ParsedNode {
                        rectangle: first_rectangle,
                        kind: first_kind,
                    } = first;
                    let first_layout = ParsedNode {
                        rectangle: first_rectangle,
                        kind: first_kind,
                    }
                    .into_layout();

                    if remaining == 1 {
                        return (first_layout, first_rectangle);
                    }

                    let (second_layout, second_rectangle) =
                        fold(children, remaining - 1, direction);
                    let combined = Rectangle::combine(first_rectangle, second_rectangle, direction)
                        .expect("tmux child geometry was validated before conversion");
                    let first_length = first_rectangle.axis_length(direction) as f64;
                    let second_length = second_rectangle.axis_length(direction) as f64;
                    let ratio = first_length / (first_length + second_length);
                    (
                        Layout::split_known_valid(direction, ratio, first_layout, second_layout),
                        combined,
                    )
                }

                let child_count = children.len();
                fold(&mut children.into_iter(), child_count, direction).0
            }
        }
    }
}

struct TmuxParser<'input> {
    input: &'input [u8],
    cursor: usize,
}

impl<'input> TmuxParser<'input> {
    fn new(input: &'input str) -> Self {
        Self {
            input: input.as_bytes(),
            cursor: 0,
        }
    }

    fn parse_node(&mut self) -> Result<ParsedNode, TmuxLayoutParseError> {
        let node_start = self.cursor;
        let width = self.parse_positive_integer("width")?;
        self.expect(b'x', "expected `x` after width")?;
        let height = self.parse_positive_integer("height")?;
        self.expect(b',', "expected `,` after height")?;
        let x = self.parse_integer("x offset")?;
        self.expect(b',', "expected `,` after x offset")?;
        let y = self.parse_integer("y offset")?;
        let rectangle = Rectangle {
            width,
            height,
            x,
            y,
        };

        match self.peek() {
            Some(b',') => {
                self.cursor += 1;
                let pane = self.parse_integer("pane index")?;
                Ok(ParsedNode {
                    rectangle,
                    kind: ParsedKind::Pane(pane),
                })
            }
            Some(b'{') => {
                self.cursor += 1;
                self.parse_children(node_start, rectangle, SplitDirection::Right, b'}')
            }
            Some(b'[') => {
                self.cursor += 1;
                self.parse_children(node_start, rectangle, SplitDirection::Down, b']')
            }
            Some(_) => Err(self.error(
                "expected pane index, `{` right-split children, or `[` down-split children",
            )),
            None => Err(self.error("unexpected end of input after node position")),
        }
    }

    fn parse_children(
        &mut self,
        node_start: usize,
        parent: Rectangle,
        direction: SplitDirection,
        closing: u8,
    ) -> Result<ParsedNode, TmuxLayoutParseError> {
        let mut children = Vec::new();
        if self.peek() == Some(closing) {
            return Err(self.error("a split container cannot be empty"));
        }

        loop {
            children.push(self.parse_node()?);
            match self.peek() {
                Some(byte) if byte == closing => {
                    self.cursor += 1;
                    break;
                }
                Some(b',') => {
                    self.cursor += 1;
                    if self.peek() == Some(closing) {
                        return Err(self.error("trailing comma in split container"));
                    }
                }
                Some(_) => {
                    return Err(self.error(format!(
                        "expected `,` or `{}` after split child",
                        char::from(closing)
                    )));
                }
                None => {
                    return Err(self.error(format!(
                        "unterminated split container; expected `{}`",
                        char::from(closing)
                    )));
                }
            }
        }

        if children.len() < 2 {
            return Err(self.error("a split container requires at least two children"));
        }

        let mut combined = children[0].rectangle;
        for child in &children[1..] {
            combined = Rectangle::combine(combined, child.rectangle, direction)
                .map_err(|message| self.error_at(node_start, message))?;
        }
        if combined != parent {
            return Err(self.error_at(
                node_start,
                format!(
                    "declared parent geometry {}x{},{},{} does not match its children",
                    parent.width, parent.height, parent.x, parent.y
                ),
            ));
        }

        Ok(ParsedNode {
            rectangle: parent,
            kind: ParsedKind::Children {
                direction,
                children,
            },
        })
    }

    fn parse_positive_integer(&mut self, label: &str) -> Result<usize, TmuxLayoutParseError> {
        let start = self.cursor;
        let value = self.parse_integer(label)?;
        if value == 0 {
            return Err(self.error_at(start, format!("{label} must be greater than zero")));
        }
        Ok(value)
    }

    fn parse_integer(&mut self, label: &str) -> Result<usize, TmuxLayoutParseError> {
        let start = self.cursor;
        while matches!(self.peek(), Some(byte) if byte.is_ascii_digit()) {
            self.cursor += 1;
        }
        if self.cursor == start {
            return Err(self.error_at(start, format!("expected decimal {label}")));
        }

        let digits = std::str::from_utf8(&self.input[start..self.cursor])
            .expect("ASCII decimal digits are valid UTF-8");
        digits
            .parse()
            .map_err(|_| self.error_at(start, format!("{label} does not fit in usize")))
    }

    fn expect(&mut self, expected: u8, message: &'static str) -> Result<(), TmuxLayoutParseError> {
        if self.peek() == Some(expected) {
            self.cursor += 1;
            Ok(())
        } else {
            Err(self.error(message))
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.cursor).copied()
    }

    fn is_finished(&self) -> bool {
        self.cursor == self.input.len()
    }

    fn error(&self, message: impl Into<String>) -> TmuxLayoutParseError {
        self.error_at(self.cursor, message)
    }

    fn error_at(
        &self,
        payload_position: usize,
        message: impl Into<String>,
    ) -> TmuxLayoutParseError {
        // Five bytes account for the four checksum digits and comma.
        TmuxLayoutParseError::new(payload_position.saturating_add(5), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1.0e-12,
            "expected {expected}, got {actual}"
        );
    }

    fn leaf_shares(layout: &Layout) -> Vec<(usize, f64)> {
        fn visit(layout: &Layout, available: f64, result: &mut Vec<(usize, f64)>) {
            match layout {
                Layout::Pane(index) => result.push((*index, available)),
                Layout::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    visit(first, available * ratio, result);
                    visit(second, available * (1.0 - ratio), result);
                }
            }
        }

        let mut result = Vec::new();
        visit(layout, 1.0, &mut result);
        result
    }

    fn with_checksum(payload: &str) -> String {
        format!("{:04x},{payload}", tmux_layout_checksum(payload))
    }

    #[test]
    fn split_constructor_and_validation_reject_invalid_trees() {
        let layout =
            Layout::split(SplitDirection::Right, 0.4, Layout::Pane(2), Layout::Pane(3)).unwrap();
        assert_eq!(layout.pane_indices(), vec![2, 3]);
        assert_eq!(layout.pane_count(), 2);
        assert!(layout.contains_pane(3));
        assert!(!layout.contains_pane(4));

        assert!(matches!(
            Layout::split(SplitDirection::Down, 0.0, Layout::Pane(0), Layout::Pane(1)),
            Err(LayoutError::InvalidRatio { .. })
        ));
        assert!(matches!(
            Layout::split(
                SplitDirection::Down,
                f64::NAN,
                Layout::Pane(0),
                Layout::Pane(1)
            ),
            Err(LayoutError::InvalidRatio { .. })
        ));
        assert_eq!(
            Layout::split(SplitDirection::Right, 0.5, Layout::Pane(7), Layout::Pane(7)),
            Err(LayoutError::DuplicatePane { index: 7 })
        );
    }

    #[test]
    fn tiled_uses_ceil_sqrt_columns_and_balanced_equal_rows() {
        let layout = Layout::default_tiled(7).unwrap();
        assert_eq!(layout.pane_indices(), (0..7).collect::<Vec<_>>());

        // ceil(sqrt(7)) is three. Three balanced rows therefore contain
        // [0,1,2], [3,4], and [5,6], and all rows get one third height.
        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = &layout
        else {
            panic!("seven tiled panes need multiple rows");
        };
        assert_eq!(*direction, SplitDirection::Down);
        assert_close(*ratio, 1.0 / 3.0);
        assert_eq!(first.pane_indices(), vec![0, 1, 2]);
        assert_eq!(second.pane_indices(), vec![3, 4, 5, 6]);

        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = second.as_ref()
        else {
            panic!("the remaining two rows need a split");
        };
        assert_eq!(*direction, SplitDirection::Down);
        assert_close(*ratio, 0.5);
        assert_eq!(first.pane_indices(), vec![3, 4]);
        assert_eq!(second.pane_indices(), vec![5, 6]);
    }

    #[test]
    fn tiled_handles_square_single_and_empty_counts() {
        assert_eq!(Layout::default_tiled(1).unwrap(), Layout::Pane(0));
        assert_eq!(
            Layout::default_tiled(4).unwrap().pane_indices(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(Layout::default_tiled(0), Err(LayoutError::Empty));
        assert_eq!(ceil_sqrt(usize::MAX), 1_usize << (usize::BITS / 2));
    }

    #[test]
    fn even_layouts_give_every_leaf_an_equal_share() {
        for (layout, expected_direction) in [
            (Layout::even_horizontal(5).unwrap(), SplitDirection::Right),
            (Layout::even_vertical(5).unwrap(), SplitDirection::Down),
        ] {
            let Layout::Split { direction, .. } = layout else {
                panic!("five panes need a split");
            };
            assert_eq!(direction, expected_direction);
            for (pane, share) in leaf_shares(&layout) {
                assert_eq!(pane, layout.pane_indices()[pane]);
                assert_close(share, 0.2);
            }
        }
    }

    #[test]
    fn large_equal_layouts_use_herdr_representable_balanced_splits() {
        fn assert_balanced_ratios(layout: &Layout) {
            if let Layout::Split {
                ratio,
                first,
                second,
                ..
            } = layout
            {
                assert!((MIN_PANE_SHARE..=MAX_PANE_SHARE).contains(ratio));
                assert_balanced_ratios(first);
                assert_balanced_ratios(second);
            }
        }

        for layout in [
            Layout::even_horizontal(11).unwrap(),
            Layout::even_vertical(64).unwrap(),
        ] {
            assert_balanced_ratios(&layout);
            let shares = leaf_shares(&layout);
            let expected_share = 1.0 / shares.len() as f64;
            for (_, share) in shares {
                assert_close(share, expected_share);
            }
        }
    }

    #[test]
    fn main_layouts_reserve_half_for_first_pane() {
        for (layout, outer, inner) in [
            (
                Layout::main_horizontal(4).unwrap(),
                SplitDirection::Down,
                SplitDirection::Right,
            ),
            (
                Layout::main_vertical(4).unwrap(),
                SplitDirection::Right,
                SplitDirection::Down,
            ),
        ] {
            let Layout::Split {
                direction,
                ratio,
                first,
                second,
            } = &layout
            else {
                panic!("four panes need a split");
            };
            assert_eq!(*direction, outer);
            assert_close(*ratio, 0.5);
            assert_eq!(first.as_ref(), &Layout::Pane(0));
            assert!(matches!(
                second.as_ref(),
                Layout::Split { direction, .. } if *direction == inner
            ));

            let shares = leaf_shares(&layout);
            assert_close(shares[0].1, 0.5);
            for (_, share) in &shares[1..] {
                assert_close(*share, 1.0 / 6.0);
            }
        }
        assert_eq!(Layout::main_vertical(1).unwrap(), Layout::Pane(0));
    }

    #[test]
    fn presets_round_trip_their_stable_names() {
        for preset in [
            LayoutPreset::Tiled,
            LayoutPreset::EvenHorizontal,
            LayoutPreset::EvenVertical,
            LayoutPreset::MainHorizontal,
            LayoutPreset::MainVertical,
        ] {
            assert_eq!(preset.as_str().parse::<LayoutPreset>().unwrap(), preset);
            assert_eq!(preset.to_string(), preset.as_str());
            assert_eq!(preset.build(3).unwrap().pane_indices(), vec![0, 1, 2]);
        }
        assert!("diagonal".parse::<LayoutPreset>().is_err());
    }

    #[test]
    fn pane_chain_splits_the_previously_created_leaf() {
        let layout = PaneChainBuilder::new(0)
            .split_right(0.6)
            .unwrap()
            .split_down(0.25)
            .unwrap()
            .build();

        assert_eq!(layout.pane_indices(), vec![0, 1, 2]);
        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = layout
        else {
            panic!("chain should have an outer split");
        };
        assert_eq!(direction, SplitDirection::Right);
        assert_close(ratio, 0.6);
        assert_eq!(*first, Layout::Pane(0));

        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = *second
        else {
            panic!("the second split should replace pane one");
        };
        assert_eq!(direction, SplitDirection::Down);
        assert_close(ratio, 0.25);
        assert_eq!(*first, Layout::Pane(1));
        assert_eq!(*second, Layout::Pane(2));
    }

    #[test]
    fn pane_chain_enforces_share_bounds_and_unique_indices() {
        assert!(PaneChainBuilder::new(0).split_right(0.1).is_ok());
        assert!(PaneChainBuilder::new(0).split_right(0.9).is_ok());
        assert!(matches!(
            PaneChainBuilder::new(0).split_right(0.099),
            Err(LayoutError::InvalidPaneShare { .. })
        ));
        assert!(matches!(
            PaneChainBuilder::new(0).split_down(f64::INFINITY),
            Err(LayoutError::InvalidPaneShare { .. })
        ));
        assert!(matches!(
            PaneChainBuilder::new(3).split_pane(3, SplitDirection::Right, 0.5),
            Err(LayoutError::DuplicatePane { index: 3 })
        ));
        assert!(matches!(
            PaneChainBuilder::new(usize::MAX).split_right(0.5),
            Err(LayoutError::PaneIndexOverflow)
        ));
    }

    #[test]
    fn one_shot_pane_chain_uses_sequential_indices() {
        let layout = Layout::pane_chain(4, SplitDirection::Right, 0.5).unwrap();
        assert_eq!(layout.pane_indices(), vec![0, 1, 2, 3]);
        assert_eq!(
            Layout::pane_chain(0, SplitDirection::Down, 0.5),
            Err(LayoutError::Empty)
        );
    }

    #[test]
    fn checksum_matches_layouts_emitted_by_tmux() {
        let payload = "120x40,0,0{40x40,0,0,0,39x40,41,0,1,39x40,81,0,2}";
        assert_eq!(tmux_layout_checksum(payload), 0x6864);
    }

    #[test]
    fn parses_tmux_nary_horizontal_layout_into_bsp() {
        let serialized = "6864,120x40,0,0{40x40,0,0,0,39x40,41,0,1,39x40,81,0,2}";
        let layout = parse_tmux_layout(serialized).unwrap();
        assert_eq!(layout.pane_indices(), vec![0, 1, 2]);

        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = layout
        else {
            panic!("three panes need a split");
        };
        assert_eq!(direction, SplitDirection::Right);
        // The second subtree spans 79 columns: pane widths 39 + 39 and its
        // own divider. The outer divider is excluded from the ratio.
        assert_close(ratio, 40.0 / (40.0 + 79.0));
        assert_eq!(*first, Layout::Pane(0));
        assert!(matches!(
            *second,
            Layout::Split {
                direction: SplitDirection::Right,
                ..
            }
        ));
    }

    #[test]
    fn parses_tmux_nested_mixed_layout() {
        let serialized = "56f7,120x40,0,0[120x19,0,0{59x19,0,0,0,60x19,60,0,1},120x20,0,20,2]";
        let layout = serialized.parse::<Layout>().unwrap();
        assert_eq!(layout.pane_indices(), vec![0, 1, 2]);

        let Layout::Split {
            direction,
            ratio,
            first,
            second,
        } = layout
        else {
            panic!("nested tmux layout needs a root split");
        };
        assert_eq!(direction, SplitDirection::Down);
        assert_close(ratio, 19.0 / 39.0);
        assert!(matches!(
            *first,
            Layout::Split {
                direction: SplitDirection::Right,
                ..
            }
        ));
        assert_eq!(*second, Layout::Pane(2));
    }

    #[test]
    fn rejects_bad_checksum_before_parsing_payload() {
        let error = parse_tmux_layout("0000,80x24,0,0,0").unwrap_err();
        assert_eq!(error.position(), 0);
        assert!(error.message().contains("checksum mismatch"));
        assert!(parse_tmux_layout("xyz,80x24,0,0,0").is_err());
    }

    #[test]
    fn rejects_invalid_tmux_geometry_with_a_valid_checksum() {
        let wrong_parent = with_checksum("121x40,0,0{40x40,0,0,0,39x40,41,0,1,39x40,81,0,2}");
        assert!(parse_tmux_layout(&wrong_parent)
            .unwrap_err()
            .message()
            .contains("parent geometry"));

        let missing_divider = with_checksum("80x24,0,0{40x24,0,0,0,39x24,40,0,1}");
        assert!(parse_tmux_layout(&missing_divider)
            .unwrap_err()
            .message()
            .contains("divider column"));

        let wrong_cross_axis = with_checksum("80x24,0,0{40x24,0,0,0,39x23,41,0,1}");
        assert!(parse_tmux_layout(&wrong_cross_axis)
            .unwrap_err()
            .message()
            .contains("equal y offsets and heights"));
    }

    #[test]
    fn rejects_malformed_tmux_structure_and_duplicate_panes() {
        for payload in [
            "80x24,0,0{80x24,0,0,0}",
            "80x24,0,0{}",
            "80x24,0,0{40x24,0,0,0,39x24,41,0,0}",
            "80x24,0,0{40x24,0,0,0,39x24,41,0,1,}",
            "80x24,0,0,0trailing",
            "0x24,0,0,0",
            "80x24,0,0,-1",
        ] {
            let serialized = with_checksum(payload);
            assert!(
                parse_tmux_layout(&serialized).is_err(),
                "unexpectedly accepted {payload}"
            );
        }
    }
}
