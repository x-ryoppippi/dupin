#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    LeftThird,
    CentreThird,
    RightThird,
    Maximise,
    Centre,
    NextDisplay,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// Flip a rectangle about the primary display’s top edge to change coordinate systems.
    pub fn flip_y(self, primary_top: f64) -> Self {
        Self {
            y: primary_top - self.y - self.height,
            ..self
        }
    }

    pub fn is_valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .into_iter()
            .all(f64::is_finite)
            && self.width > 0.0
            && self.height > 0.0
    }

    fn overlap(self, other: Self) -> f64 {
        let width = (self.x + self.width).min(other.x + other.width) - self.x.max(other.x);
        let height = (self.y + self.height).min(other.y + other.height) - self.y.max(other.y);
        width.max(0.0) * height.max(0.0)
    }

    fn tile(self, column: u8, columns: u8, row: u8, rows: u8) -> Self {
        let edge = |length: f64, i: u8, n: u8| (length * f64::from(i) / f64::from(n)).round();
        let x = edge(self.width, column, columns);
        let y = edge(self.height, row, rows);
        Self::new(
            self.x + x,
            self.y + y,
            edge(self.width, column + 1, columns) - x,
            edge(self.height, row + 1, rows) - y,
        )
    }
}

/// Display bounds in top-left coordinates, with the primary display first.
pub struct Display {
    pub frame: Frame,
    pub visible: Frame,
}

pub fn target_frame(layout: Layout, window: Frame, displays: &[Display]) -> Option<Frame> {
    let index = displays
        .iter()
        .enumerate()
        .map(|(index, display)| (index, display.frame.overlap(window)))
        .reduce(|best, candidate| if candidate.1 > best.1 { candidate } else { best })?
        .0;
    let visible = displays[index].visible;
    let target = match layout {
        Layout::LeftHalf => visible.tile(0, 2, 0, 1),
        Layout::RightHalf => visible.tile(1, 2, 0, 1),
        Layout::TopHalf => visible.tile(0, 1, 0, 2),
        Layout::BottomHalf => visible.tile(0, 1, 1, 2),
        Layout::LeftThird => visible.tile(0, 3, 0, 1),
        Layout::CentreThird => visible.tile(1, 3, 0, 1),
        Layout::RightThird => visible.tile(2, 3, 0, 1),
        Layout::Maximise => visible,
        Layout::Centre => {
            let width = window.width.min(visible.width);
            let height = window.height.min(visible.height);
            Frame::new(
                visible.x + (visible.width - width) / 2.0,
                visible.y + (visible.height - height) / 2.0,
                width,
                height,
            )
        }
        Layout::NextDisplay if displays.len() == 1 => window,
        Layout::NextDisplay => {
            let next = displays[(index + 1) % displays.len()].visible;
            let width = window.width.min(next.width);
            let height = window.height.min(next.height);
            let fraction = |offset: f64, travel: f64| {
                if travel > 0.0 {
                    (offset / travel).clamp(0.0, 1.0)
                } else {
                    0.5
                }
            };
            let x = fraction(window.x - visible.x, visible.width - window.width);
            let y = fraction(window.y - visible.y, visible.height - window.height);
            Frame::new(
                next.x + x * (next.width - width),
                next.y + y * (next.height - height),
                width,
                height,
            )
        }
    };
    target.is_valid().then_some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(frame: Frame, visible: Frame, primary_top: f64) -> Display {
        Display {
            frame: frame.flip_y(primary_top),
            visible: visible.flip_y(primary_top),
        }
    }

    #[test]
    fn all_tiles_use_the_visible_frame_and_share_odd_boundaries() {
        let visible = Frame::new(10.0, 25.0, 1001.0, 799.0);
        let screens = [Display {
            frame: Frame::new(0.0, 0.0, 1024.0, 900.0),
            visible,
        }];
        let window = Frame::new(50.0, 60.0, 220.0, 180.0);
        for (layout, expected) in [
            (Layout::LeftHalf, Frame::new(10.0, 25.0, 501.0, 799.0)),
            (Layout::RightHalf, Frame::new(511.0, 25.0, 500.0, 799.0)),
            (Layout::TopHalf, Frame::new(10.0, 25.0, 1001.0, 400.0)),
            (Layout::BottomHalf, Frame::new(10.0, 425.0, 1001.0, 399.0)),
            (Layout::LeftThird, Frame::new(10.0, 25.0, 334.0, 799.0)),
            (Layout::CentreThird, Frame::new(344.0, 25.0, 333.0, 799.0)),
            (Layout::RightThird, Frame::new(677.0, 25.0, 334.0, 799.0)),
            (Layout::Maximise, visible),
            (Layout::Centre, Frame::new(400.5, 334.5, 220.0, 180.0)),
            (Layout::NextDisplay, window),
        ] {
            assert_eq!(
                target_frame(layout, window, &screens),
                Some(expected),
                "{layout:?}"
            );
        }
    }

    #[test]
    fn a_straddling_window_uses_the_display_with_most_overlap() {
        let screens = [
            display(
                Frame::new(0.0, 0.0, 1440.0, 900.0),
                Frame::new(0.0, 32.0, 1440.0, 844.0),
                900.0,
            ),
            display(
                Frame::new(-1200.0, 100.0, 1200.0, 1000.0),
                Frame::new(-1180.0, 100.0, 1180.0, 976.0),
                900.0,
            ),
        ];
        let window = Frame::new(-300.0, 10.0, 360.0, 300.0);
        assert_eq!(
            target_frame(Layout::TopHalf, window, &screens),
            Some(Frame::new(-1180.0, -176.0, 1180.0, 488.0)),
        );
    }

    #[test]
    fn next_display_cycles_across_vertical_and_offset_displays() {
        let screens = [
            display(
                Frame::new(0.0, 0.0, 1200.0, 800.0),
                Frame::new(0.0, 0.0, 1200.0, 780.0),
                800.0,
            ),
            display(
                Frame::new(200.0, 800.0, 800.0, 600.0),
                Frame::new(200.0, 800.0, 800.0, 580.0),
                800.0,
            ),
            display(
                Frame::new(-100.0, -500.0, 700.0, 500.0),
                Frame::new(-100.0, -500.0, 700.0, 480.0),
                800.0,
            ),
        ];
        let window = Frame::new(400.0, 260.0, 400.0, 300.0);
        let above = Frame::new(400.0, -440.0, 400.0, 300.0);
        let below = Frame::new(50.0, 910.0, 400.0, 300.0);
        assert_eq!(target_frame(Layout::NextDisplay, window, &screens), Some(above));
        assert_eq!(target_frame(Layout::NextDisplay, above, &screens), Some(below));
        assert_eq!(target_frame(Layout::NextDisplay, below, &screens), Some(window));
        assert_eq!(
            target_frame(
                Layout::NextDisplay,
                Frame::new(0.0, 20.0, 1200.0, 780.0),
                &screens
            ),
            Some(screens[1].visible)
        );
    }
}
