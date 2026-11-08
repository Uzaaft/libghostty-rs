//! Name a grid position in the active area, viewport, screen, or history.
//!
//! A [`Point`] names a position in the terminal grid, in one of four
//! coordinate systems: the active area, the viewport, the full screen, or the
//! scrollback history.
//!
//! You need one to find a cell. The same row has a different number in each
//! system, so pick the one that matches what you're asking. Resolve a point
//! to a cell with [`Terminal::grid_ref`](crate::Terminal::grid_ref).
use crate::ffi;

/// A point in the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Point {
    /// Active area where the cursor can move.
    Active(Coordinate),
    /// Visible viewport (changes when scrolled).
    Viewport(Coordinate),
    /// Full screen including scrollback.
    Screen(Coordinate),
    /// Scrollback history only (before active area).
    History(Coordinate),
}

impl From<Point> for ffi::Point {
    fn from(value: Point) -> Self {
        match value {
            Point::Active(coord) => Self {
                tag: ffi::PointTag::ACTIVE,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::Viewport(coord) => Self {
                tag: ffi::PointTag::VIEWPORT,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::Screen(coord) => Self {
                tag: ffi::PointTag::SCREEN,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
            Point::History(coord) => Self {
                tag: ffi::PointTag::HISTORY,
                value: ffi::PointValue {
                    coordinate: coord.into(),
                },
            },
        }
    }
}

/// A coordinate space for converting grid references back to points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    /// Active area where the cursor can move.
    Active,
    /// Visible viewport, which changes when scrolled.
    Viewport,
    /// Full screen including scrollback.
    Screen,
    /// Scrollback history only, before the active area.
    History,
}

impl Space {
    pub(crate) fn into_raw(self) -> ffi::PointTag::Type {
        match self {
            Self::Active => ffi::PointTag::ACTIVE,
            Self::Viewport => ffi::PointTag::VIEWPORT,
            Self::Screen => ffi::PointTag::SCREEN,
            Self::History => ffi::PointTag::HISTORY,
        }
    }
}

/// A coordinate in the terminal grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Coordinate {
    /// Column (0-indexed).
    pub x: u16,
    /// Row (0-indexed). May exceed page size for screen/history tags.
    pub y: u32,
}
impl From<Coordinate> for ffi::PointCoordinate {
    fn from(value: Coordinate) -> Self {
        let Coordinate { x, y } = value;
        Self { x, y }
    }
}
impl From<ffi::PointCoordinate> for Coordinate {
    fn from(value: ffi::PointCoordinate) -> Self {
        let ffi::PointCoordinate { x, y } = value;
        Self { x, y }
    }
}
