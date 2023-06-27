// SPDX-License-Identifier: LGPL-3.0-or-later OR MPL-2.0
// This file is a part of `piet-hardware`.
//
// `piet-hardware` is free software: you can redistribute it and/or modify it under the
// terms of either:
//
// * GNU Lesser General Public License as published by the Free Software Foundation, either
//   version 3 of the License, or (at your option) any later version.
// * Mozilla Public License as published by the Mozilla Foundation, version 2.
// * The Patron License (https://github.com/notgull/piet-hardware/blob/main/LICENSE-PATRON.md)
//   for sponsors and contributors, who can ignore the copyleft provisions of the above licenses
//   for this project.
//
// `piet-hardware` is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR
// PURPOSE. See the GNU Lesser General Public License or the Mozilla Public License for more
// details.
//
// You should have received a copy of the GNU Lesser General Public License and the Mozilla
// Public License along with `piet-hardware`. If not, see <https://www.gnu.org/licenses/>.

//! The rasterizer, powered by `lyon_tessellation`.

use super::gpu_backend::Vertex;
use super::ResultExt;

use arrayvec::ArrayVec;

use tiny_skia::PathSegment;

use lyon_tessellation::path::{Event, PathEvent};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};

use piet::kurbo::{BezPath, PathEl, Point, Rect, Shape};
use piet::{Color, Error as Pierror, LineCap, LineJoin};

pub(crate) struct Rasterizer {
    /// Buffers for tessellating the path.
    buffers: VertexBuffers<Vertex, u32>,

    /// The fill tessellator.
    fill_tessellator: FillTessellator,

    /// The stroke tessellator.
    stroke_tessellator: StrokeTessellator,
}

impl Rasterizer {
    /// Create a new rasterizer.
    pub(crate) fn new() -> Self {
        Self {
            buffers: VertexBuffers::new(),
            fill_tessellator: FillTessellator::new(),
            stroke_tessellator: StrokeTessellator::new(),
        }
    }

    /// Get a reference to the vertex buffer.
    pub(crate) fn vertices(&self) -> &[Vertex] {
        &self.buffers.vertices
    }

    /// Get a reference to the index buffer.
    pub(crate) fn indices(&self) -> &[u32] {
        &self.buffers.indices
    }

    /// Clear the rasterizer's buffers.
    pub(crate) fn clear(&mut self) {
        self.buffers.vertices.clear();
        self.buffers.indices.clear();
    }

    /// Tessellate a series of rectangles.
    pub(crate) fn fill_rects(&mut self, rects: impl IntoIterator<Item = TessRect>) {
        // Get the vertices associated with the rectangles.
        let mut rect_count = 0;
        let mut vertices = |pos_rect: Rect, uv_rect: Rect, color: piet::Color| {
            rect_count += 1;
            let cast = |x: f64| x as f32;
            let (r, g, b, a) = color.as_rgba8();
            let color = [r, g, b, a];

            [
                Vertex {
                    pos: [cast(pos_rect.x0), cast(pos_rect.y0)],
                    uv: [cast(uv_rect.x0), cast(uv_rect.y0)],
                    color,
                },
                Vertex {
                    pos: [cast(pos_rect.x1), cast(pos_rect.y0)],
                    uv: [cast(uv_rect.x1), cast(uv_rect.y0)],
                    color,
                },
                Vertex {
                    pos: [cast(pos_rect.x1), cast(pos_rect.y1)],
                    uv: [cast(uv_rect.x1), cast(uv_rect.y1)],
                    color,
                },
                Vertex {
                    pos: [cast(pos_rect.x0), cast(pos_rect.y1)],
                    uv: [cast(uv_rect.x0), cast(uv_rect.y1)],
                    color,
                },
            ]
        };

        // Add the vertices to the buffers.
        self.buffers
            .vertices
            .extend(rects.into_iter().flat_map(|tess| {
                let TessRect { pos, uv, color } = tess;
                vertices(pos, uv, color)
            }));
        self.buffers.indices.extend((0..rect_count).flat_map(|i| {
            let base = i * 4;
            [base, base + 1, base + 2, base, base + 2, base + 3]
        }));
    }

    /// Tessellate a filled shape.
    pub(crate) fn fill_shape(
        &mut self,
        shape: impl Shape,
        mode: FillRule,
        tolerance: f64,
        cvt_vertex: impl Fn(FillVertex<'_>) -> Vertex,
    ) -> Result<(), Pierror> {
        // Create a new buffers builder.
        let mut builder = BuffersBuilder::new(&mut self.buffers, move |vertex: FillVertex<'_>| {
            cvt_vertex(vertex)
        });

        // Create fill options.
        let mut options = FillOptions::default();
        options.fill_rule = mode;
        options.tolerance = tolerance as f32;

        // Fill the shape.
        self.fill_tessellator
            .tessellate(
                shape_to_lyon_path(&shape, tolerance),
                &options,
                &mut builder,
            )
            .piet_err()
    }

    /// Tessellate the stroke of a shape.
    pub(crate) fn stroke_shape(
        &mut self,
        shape: impl Shape,
        tolerance: f64,
        width: f64,
        style: &piet::StrokeStyle,
        cvt_vertex: impl Fn(StrokeVertex<'_, '_>) -> Vertex,
        cvt_fill_vertex: impl Fn(FillVertex<'_>) -> Vertex,
    ) -> Result<(), Pierror> {
        // lyon_tesselation does not support dashing. If this is a dashed path, use kurbo
        // to calculate the dash path.
        if !style.dash_pattern.is_empty() {
            let mut stroke = kurbo::Stroke::new(width);
            let cap = match style.line_cap {
                LineCap::Butt => kurbo::Cap::Butt,
                LineCap::Round => kurbo::Cap::Round,
                LineCap::Square => kurbo::Cap::Square,
            };
            stroke = stroke.with_caps(cap);
            stroke = match style.line_join {
                LineJoin::Bevel => stroke.with_join(kurbo::Join::Bevel),
                LineJoin::Round => stroke.with_join(kurbo::Join::Round),
                LineJoin::Miter { limit } => {
                    stroke.with_miter_limit(limit).with_join(kurbo::Join::Miter)
                }
            };
            stroke = stroke.with_dashes(style.dash_offset, style.dash_pattern.iter().copied());

            // Stroke out and fill it.
            let filled_path = kurbo::stroke(shape.path_elements(tolerance), &stroke, tolerance);

            return self.fill_shape(filled_path, FillRule::NonZero, tolerance, cvt_fill_vertex);
        }

        // Create a new buffers builder.
        let mut builder =
            BuffersBuilder::new(&mut self.buffers, move |vertex: StrokeVertex<'_, '_>| {
                cvt_vertex(vertex)
            });

        let cvt_line_cap = |cap: LineCap| match cap {
            LineCap::Butt => lyon_tessellation::LineCap::Butt,
            LineCap::Round => lyon_tessellation::LineCap::Round,
            LineCap::Square => lyon_tessellation::LineCap::Square,
        };

        // Create stroke options.
        let mut options = StrokeOptions::default();
        options.tolerance = tolerance as f32;
        options.line_width = width as f32;
        options.start_cap = cvt_line_cap(style.line_cap);
        options.end_cap = cvt_line_cap(style.line_cap);
        options.line_join = match style.line_join {
            LineJoin::Bevel => lyon_tessellation::LineJoin::Bevel,
            LineJoin::Round => lyon_tessellation::LineJoin::Round,
            LineJoin::Miter { limit } => {
                options.miter_limit = limit as f32;
                lyon_tessellation::LineJoin::Miter
            }
        };

        // Fill the shape.
        self.stroke_tessellator
            .tessellate(
                shape_to_lyon_path(&shape, tolerance),
                &options,
                &mut builder,
            )
            .piet_err()
    }
}

/// A rectangle to be tessellated.
#[derive(Debug, Clone)]
pub(crate) struct TessRect {
    /// The rectangle to be tessellated.
    pub(crate) pos: Rect,

    /// The UV coordinates of the rectangle.
    pub(crate) uv: Rect,

    /// The color of the rectangle.
    pub(crate) color: Color,
}

fn shape_to_lyon_path(shape: &impl Shape, tolerance: f64) -> impl Iterator<Item = PathEvent> + '_ {
    use std::iter::Fuse;

    fn convert_point(pt: Point) -> lyon_tessellation::path::geom::Point<f32> {
        let (x, y): (f64, f64) = pt.into();
        [x as f32, y as f32].into()
    }

    struct PathConverter<I> {
        /// The iterator over `kurbo` `PathEl`s.
        iter: Fuse<I>,

        /// The last point that we processed.
        last: Option<Point>,

        /// The first point of the current subpath.
        first: Option<Point>,

        // Whether or not we need to close the path.
        needs_close: bool,
    }

    impl<I: Iterator<Item = PathEl>> Iterator for PathConverter<I> {
        type Item = ArrayVec<PathEvent, 2>;

        fn next(&mut self) -> Option<Self::Item> {
            let close = |this: &mut PathConverter<I>, close| {
                if let (Some(first), Some(last)) = (this.first.take(), this.last.take()) {
                    if (!approx_eq(first.x, last.x) || !approx_eq(first.y, last.y))
                        || (this.needs_close || close)
                    {
                        this.needs_close = false;
                        return Some(Event::End {
                            last: convert_point(last),
                            first: convert_point(first),
                            close,
                        });
                    }
                }

                None
            };

            let el = match self.iter.next() {
                Some(el) => el,
                None => {
                    // If we're at the end of the iterator, we need to close the path.
                    return close(self, false).map(one);
                }
            };

            match el {
                PathEl::MoveTo(pt) => {
                    // Close if we need to.
                    let close = close(self, false);

                    // Set the first point.
                    self.first = Some(pt);
                    self.last = Some(pt);

                    let mut v = ArrayVec::new();
                    v.extend(close);
                    v.push(Event::Begin {
                        at: convert_point(pt),
                    });
                    Some(v)
                }

                PathEl::LineTo(pt) => {
                    self.needs_close = true;
                    let from = self.last.replace(pt).expect("last point should be set");

                    Some(one(Event::Line {
                        from: convert_point(from),
                        to: convert_point(pt),
                    }))
                }

                PathEl::QuadTo(ctrl1, pt) => {
                    self.needs_close = true;
                    let from = self.last.replace(pt).expect("last point should be set");

                    Some(one(Event::Quadratic {
                        from: convert_point(from),
                        ctrl: convert_point(ctrl1),
                        to: convert_point(pt),
                    }))
                }

                PathEl::CurveTo(ctrl1, ctrl2, pt) => {
                    self.needs_close = true;
                    let from = self.last.replace(pt).expect("last point should be set");

                    Some(one(Event::Cubic {
                        from: convert_point(from),
                        ctrl1: convert_point(ctrl1),
                        ctrl2: convert_point(ctrl2),
                        to: convert_point(pt),
                    }))
                }

                PathEl::ClosePath => {
                    let mut v = ArrayVec::new();
                    v.extend(close(self, true));
                    Some(v)
                }
            }
        }
    }

    PathConverter {
        iter: shape.path_elements(tolerance).fuse(),
        last: None,
        first: None,
        needs_close: false,
    }
    .flatten()
}

struct TinySkiaPathAsShape(tiny_skia::Path);

impl TinySkiaPathAsShape {
    fn ugly_convert_to_bezpath(&self) -> BezPath {
        let mut path = BezPath::new();

        for elem in self.0.segments() {
            path.push(ts_pathseg_to_path_el(elem));
        }

        path
    }
}

impl Shape for TinySkiaPathAsShape {
    type PathElementsIter<'iter> = TinySkiaPathIter<'iter>;

    fn path_elements(&self, _tolerance: f64) -> Self::PathElementsIter<'_> {
        TinySkiaPathIter(self.0.segments())
    }

    fn area(&self) -> f64 {
        self.ugly_convert_to_bezpath().area()
    }

    fn perimeter(&self, accuracy: f64) -> f64 {
        self.ugly_convert_to_bezpath().perimeter(accuracy)
    }

    fn winding(&self, pt: kurbo::Point) -> i32 {
        self.ugly_convert_to_bezpath().winding(pt)
    }

    fn bounding_box(&self) -> kurbo::Rect {
        self.ugly_convert_to_bezpath().bounding_box()
    }
}

struct TinySkiaPathIter<'a>(tiny_skia::PathSegmentsIter<'a>);

impl Iterator for TinySkiaPathIter<'_> {
    type Item = PathEl;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(ts_pathseg_to_path_el)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }

    fn count(self) -> usize {
        self.0.count()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.0.nth(n).map(ts_pathseg_to_path_el)
    }

    fn last(self) -> Option<Self::Item> {
        self.0.last().map(ts_pathseg_to_path_el)
    }

    fn collect<B: std::iter::FromIterator<Self::Item>>(self) -> B {
        self.0.map(ts_pathseg_to_path_el).collect()
    }

    fn fold<Acc, G>(self, init: Acc, mut g: G) -> Acc
    where
        G: FnMut(Acc, Self::Item) -> Acc,
    {
        self.0
            .fold(init, |acc, seg| g(acc, ts_pathseg_to_path_el(seg)))
    }
}

fn ts_pathseg_to_path_el(pathseg: PathSegment) -> PathEl {
    match pathseg {
        PathSegment::MoveTo(p) => PathEl::MoveTo(cvt_ts_point(p)),
        PathSegment::LineTo(p) => PathEl::LineTo(cvt_ts_point(p)),
        PathSegment::QuadTo(p1, p2) => PathEl::QuadTo(cvt_ts_point(p1), cvt_ts_point(p2)),
        PathSegment::CubicTo(p1, p2, p3) => {
            PathEl::CurveTo(cvt_ts_point(p1), cvt_ts_point(p2), cvt_ts_point(p3))
        }
        PathSegment::Close => PathEl::ClosePath,
    }
}

fn cvt_ts_point(p: tiny_skia::Point) -> kurbo::Point {
    kurbo::Point::new(p.x as f64, p.y as f64)
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.01
}

fn one(p: PathEvent) -> ArrayVec<PathEvent, 2> {
    let mut v = ArrayVec::new();
    v.push(p);
    v
}
