pub mod pos2;
pub mod rect;
pub mod vec2;

use std::ops::*;

use shrinkwraprs::Shrinkwrap;

/// # Coordinate systems
///
/// ## Screen
/// The area of your display; units are points.
///
/// ## Canvas
/// The editing area; units are canvas pixels.
///
/// ## Grid
/// The editing area divided into square cells; units are cell indices.

/// Forwards the `emath` API we actually call onto a coordinate-space newtype, so a value can't
/// silently cross between screen, canvas and grid space. Only the methods in use are forwarded;
/// add one here when a call site needs it.
macro_rules! wrap_coordinates {
    ($wrapper:ident) => {
        impl $wrapper<emath::Vec2> {
            pub const ZERO: Self = Self::new(0., 0.);

            #[inline(always)]
            pub const fn new(x: f32, y: f32) -> Self {
                Self(emath::Vec2::new(x, y))
            }

            #[inline(always)]
            pub fn splat(v: f32) -> Self {
                Self(emath::Vec2::splat(v))
            }

            #[inline(always)]
            pub fn to_pos2(self) -> $wrapper<emath::Pos2> {
                $wrapper(self.0.to_pos2())
            }

            #[inline(always)]
            pub fn relative_to(self, other: Self) -> Self {
                Self(self.0 - other.0)
            }

            #[inline(always)]
            pub fn floor(self) -> Self {
                Self(self.0.floor())
            }

            #[inline(always)]
            pub fn round(self) -> Self {
                Self(self.0.round())
            }

            #[inline(always)]
            pub fn ceil(self) -> Self {
                Self(self.0.ceil())
            }

            #[inline]
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            #[inline]
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            #[inline]
            #[must_use]
            pub fn clamp(self, min: Self, max: Self) -> Self {
                Self(self.0.clamp(min.0, max.0))
            }
        }

        impl Neg for $wrapper<emath::Vec2> {
            type Output = Self;

            fn neg(self) -> Self::Output {
                Self(self.0.neg())
            }
        }

        impl Mul for $wrapper<emath::Vec2> {
            type Output = Self;

            fn mul(self, rhs: Self) -> Self::Output {
                Self(self.0.mul(rhs.0))
            }
        }

        impl MulAssign<f32> for $wrapper<emath::Vec2> {
            fn mul_assign(&mut self, rhs: f32) {
                self.0.mul_assign(rhs);
            }
        }

        impl Mul<f32> for $wrapper<emath::Vec2> {
            type Output = Self;

            fn mul(self, rhs: f32) -> Self::Output {
                Self(self.0.mul(rhs))
            }
        }

        impl Mul<$wrapper<emath::Vec2>> for f32 {
            type Output = $wrapper<emath::Vec2>;

            fn mul(self, rhs: $wrapper<emath::Vec2>) -> Self::Output {
                $wrapper(self.mul(rhs.0))
            }
        }

        impl Div for $wrapper<emath::Vec2> {
            type Output = Self;

            fn div(self, rhs: Self) -> Self::Output {
                Self(self.0.div(rhs.0))
            }
        }

        impl Div<f32> for $wrapper<emath::Vec2> {
            type Output = Self;

            fn div(self, rhs: f32) -> Self::Output {
                Self(self.0.div(rhs))
            }
        }

        impl AddAssign for $wrapper<emath::Vec2> {
            fn add_assign(&mut self, rhs: Self) {
                self.0.add_assign(rhs.0);
            }
        }

        impl SubAssign for $wrapper<emath::Vec2> {
            fn sub_assign(&mut self, rhs: Self) {
                self.0.sub_assign(rhs.0);
            }
        }

        impl Add for $wrapper<emath::Vec2> {
            type Output = Self;

            fn add(self, rhs: Self) -> Self::Output {
                Self(self.0.add(rhs.0))
            }
        }

        impl Sub for $wrapper<emath::Vec2> {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self::Output {
                Self(self.0.sub(rhs.0))
            }
        }

        impl $wrapper<emath::Pos2> {
            pub const ZERO: Self = Self::new(0., 0.);

            #[inline(always)]
            pub const fn new(x: f32, y: f32) -> Self {
                Self(emath::Pos2::new(x, y))
            }

            #[inline(always)]
            pub fn to_vec2(self) -> $wrapper<emath::Vec2> {
                $wrapper(self.0.to_vec2())
            }

            #[inline(always)]
            pub fn relative_to(self, other: Self) -> Self {
                Self(self.0 - other.0.to_vec2())
            }

            #[inline(always)]
            pub fn floor(self) -> Self {
                Self(self.0.floor())
            }

            #[inline(always)]
            pub fn round(self) -> Self {
                Self(self.0.round())
            }

            #[inline(always)]
            pub fn ceil(self) -> Self {
                Self(self.0.ceil())
            }

            #[inline]
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            #[inline]
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            #[inline]
            #[must_use]
            pub fn clamp(self, min: Self, max: Self) -> Self {
                Self(self.0.clamp(min.0, max.0))
            }
        }

        impl Sub for $wrapper<emath::Pos2> {
            type Output = $wrapper<emath::Vec2>;

            fn sub(self, rhs: Self) -> Self::Output {
                $wrapper(self.0.sub(rhs.0))
            }
        }

        impl AddAssign<$wrapper<emath::Vec2>> for $wrapper<emath::Pos2> {
            fn add_assign(&mut self, rhs: $wrapper<emath::Vec2>) {
                self.0.add_assign(rhs.0);
            }
        }

        impl SubAssign<$wrapper<emath::Vec2>> for $wrapper<emath::Pos2> {
            fn sub_assign(&mut self, rhs: $wrapper<emath::Vec2>) {
                self.0.sub_assign(rhs.0);
            }
        }

        impl Add<$wrapper<emath::Vec2>> for $wrapper<emath::Pos2> {
            type Output = Self;

            fn add(self, rhs: $wrapper<emath::Vec2>) -> Self::Output {
                Self(self.0.add(rhs.0))
            }
        }

        impl Sub<$wrapper<emath::Vec2>> for $wrapper<emath::Pos2> {
            type Output = Self;

            fn sub(self, rhs: $wrapper<emath::Vec2>) -> Self::Output {
                Self(self.0.sub(rhs.0))
            }
        }

        impl $wrapper<emath::Rect> {
            #[inline(always)]
            pub const fn from_min_max(min: $wrapper<emath::Pos2>, max: $wrapper<emath::Pos2>) -> Self {
                Self(emath::Rect { min: min.0, max: max.0 })
            }

            #[inline(always)]
            pub fn from_min_size(min: $wrapper<emath::Pos2>, size: $wrapper<emath::Vec2>) -> Self {
                Self(emath::Rect::from_min_size(min.0, size.0))
            }

            #[inline(always)]
            pub fn from_center_size(center: $wrapper<emath::Pos2>, size: $wrapper<emath::Vec2>) -> Self {
                Self(emath::Rect::from_center_size(center.0, size.0))
            }

            #[inline(always)]
            pub fn from_two_pos(a: $wrapper<emath::Pos2>, b: $wrapper<emath::Pos2>) -> Self {
                Self(emath::Rect::from_two_pos(a.0, b.0))
            }

            #[inline(always)]
            #[must_use]
            pub fn expand(self, amnt: f32) -> Self {
                Self(self.0.expand(amnt))
            }

            #[inline(always)]
            #[must_use]
            pub fn translate(self, amnt: $wrapper<emath::Vec2>) -> Self {
                Self(self.0.translate(amnt.0))
            }

            #[inline(always)]
            pub fn intersects(self, other: Self) -> bool {
                self.0.intersects(other.0)
            }

            #[inline(always)]
            pub fn contains(self, other: $wrapper<emath::Pos2>) -> bool {
                self.0.contains(other.0)
            }

            #[inline(always)]
            pub fn clamp(self, p: $wrapper<emath::Pos2>) -> $wrapper<emath::Pos2> {
                $wrapper(self.0.clamp(p.0))
            }

            #[inline(always)]
            #[must_use]
            pub fn union(self, other: Self) -> Self {
                Self(self.0.union(other.0))
            }

            #[inline(always)]
            #[must_use]
            pub fn intersect(self, other: Self) -> Self {
                Self(self.0.intersect(other.0))
            }

            #[inline(always)]
            pub fn center(self) -> $wrapper<emath::Pos2> {
                $wrapper(self.0.center())
            }

            #[inline(always)]
            pub fn size(self) -> $wrapper<emath::Vec2> {
                $wrapper(self.0.size())
            }

            #[inline(always)]
            pub fn left_top(&self) -> $wrapper<emath::Pos2> {
                $wrapper(self.0.left_top())
            }

            #[inline(always)]
            pub fn right_bottom(&self) -> $wrapper<emath::Pos2> {
                $wrapper(self.0.right_bottom())
            }
        }
    };
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd, Shrinkwrap)]
#[shrinkwrap(mutable)]
pub struct OnScreen<T>(pub T);

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd, Shrinkwrap)]
#[shrinkwrap(mutable)]
pub struct OnCanvas<T>(pub T);

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Ord, PartialOrd, Shrinkwrap)]
#[shrinkwrap(mutable)]
pub struct OnGrid<T>(pub T);

wrap_coordinates!(OnScreen);
wrap_coordinates!(OnCanvas);
wrap_coordinates!(OnGrid);
