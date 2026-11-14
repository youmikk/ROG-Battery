//! 设备面板的 24×24 线性图标，统一圆头、圆角和留白。
use super::gfx::{Painter, R};
use crate::tray_prefs::TrayKind;

#[derive(Clone, Copy)]
pub enum Icon {
    Device(TrayKind),
    Refresh,
    Settings,
    Bolt,
    Chevron,
    Speaker,
    Microphone,
}

pub fn draw(p: &mut Painter, icon: Icon, r: R, color: u32, rotation: f32) {
    let width = match icon {
        Icon::Device(_) => 1.5,
        _ => 1.6,
    };
    let mut g = Glyph {
        p,
        r,
        color,
        width,
        rotation,
    };
    match icon {
        Icon::Device(TrayKind::Mouse) => {
            g.rounded(5.5, 2.5, 13.0, 19.0, 6.5);
            g.path(&[(12.0, 6.0), (12.0, 10.0)]);
        }
        Icon::Device(TrayKind::Keyboard) => {
            g.rounded(2.0, 5.5, 20.0, 13.0, 2.5);
            for y in [9.0, 12.0] {
                for x in [6.0, 9.5, 13.0, 16.5] {
                    g.path(&[(x, y), (x + 0.5, y)]);
                }
            }
            g.path(&[(8.0, 15.0), (16.0, 15.0)]);
        }
        Icon::Device(TrayKind::Headset) => {
            let mut points = vec![(4.0, 14.0), (4.0, 11.0)];
            arc(&mut points, (12.0, 11.0), 8.0, 180.0, 180.0);
            points.push((20.0, 14.0));
            g.path(&points);
            let mut mic = vec![(19.0, 18.0), (19.0, 19.0)];
            arc(&mut mic, (17.0, 19.0), 2.0, 0.0, 90.0);
            mic.push((14.0, 21.0));
            g.path(&mic);
            g.rounded(3.0, 12.0, 4.0, 7.0, 2.0);
            g.rounded(17.0, 12.0, 4.0, 7.0, 2.0);
        }
        Icon::Device(TrayKind::Gamepad) => {
            let mut body = vec![(8.0, 6.0), (16.0, 6.0)];
            cubic(&mut body, (19.0, 6.0), (20.0, 8.0), (20.6, 11.0));
            body.push((21.6, 16.5));
            cubic(&mut body, (22.0, 19.0), (19.8, 20.2), (18.0, 18.5));
            body.extend([(15.5, 16.2), (8.5, 16.2), (6.0, 18.5)]);
            cubic(&mut body, (4.2, 20.2), (2.0, 19.0), (2.4, 16.5));
            body.push((3.4, 11.0));
            cubic(&mut body, (4.0, 8.0), (5.0, 6.0), (8.0, 6.0));
            g.path(&body);
            g.path(&[(7.0, 9.0), (7.0, 14.0)]);
            g.path(&[(4.5, 11.5), (9.5, 11.5)]);
            g.path(&[(16.0, 10.0), (16.1, 10.0)]);
            g.path(&[(18.5, 12.5), (18.6, 12.5)]);
        }
        Icon::Device(_) => {
            g.rounded(5.0, 2.5, 14.0, 19.0, 2.5);
            g.path(&[(10.0, 18.0), (14.0, 18.0)]);
        }
        Icon::Settings => {
            let mut points = vec![(9.6, 3.0)];
            for (dx, dy) in [
                (-0.7, 2.1),
                (-2.0, 0.9),
                (-2.0, -0.5),
                (-2.3, 4.0),
                (1.5, 1.6),
                (0.0, 2.0),
                (-1.5, 1.5),
                (2.3, 4.0),
                (2.0, -0.5),
                (2.0, 1.0),
                (0.7, 2.0),
                (4.8, 0.0),
                (0.7, -2.1),
                (2.0, -0.9),
                (2.0, 0.5),
                (2.3, -4.0),
                (-1.5, -1.6),
                (0.0, -2.0),
                (1.5, -1.5),
                (-2.3, -4.0),
                (-2.0, 0.5),
                (-2.0, -1.0),
                (-0.7, -2.0),
            ] {
                let (x, y) = *points.last().unwrap();
                points.push((x + dx, y + dy));
            }
            points.push(points[0]);
            g.path(&points);
            g.circle(12.0, 12.0, 3.0);
        }
        Icon::Refresh => {
            g.path(&[(20.0, 6.0), (20.0, 11.0), (15.0, 11.0)]);
            // SVG 的 A7.3 7.3 0 1 0：圆心由两端点和半径确定。
            let (start, end, radius) = ((19.1_f32, 10.0_f32), (19.0_f32, 16.0_f32), 7.3_f32);
            let (dx, dy) = (end.0 - start.0, end.1 - start.1);
            let len = dx.hypot(dy);
            let distance = (radius * radius - len * len / 4.0).sqrt();
            let center = (
                (start.0 + end.0) / 2.0 - dy / len * distance,
                (start.1 + end.1) / 2.0 + dx / len * distance,
            );
            let a = (start.1 - center.1).atan2(start.0 - center.0).to_degrees();
            let b = (end.1 - center.1).atan2(end.0 - center.0).to_degrees();
            let mut points = Vec::new();
            arc(&mut points, center, radius, a, b - a - 360.0);
            g.path(&points);
        }
        Icon::Bolt => g.path(&[
            (13.5, 2.0),
            (4.5, 14.0),
            (11.0, 14.0),
            (10.5, 22.0),
            (19.5, 10.0),
            (13.0, 10.0),
            (13.5, 2.0),
        ]),
        Icon::Chevron => g.path(&[(9.0, 5.0), (16.0, 12.0), (9.0, 19.0)]),
        Icon::Speaker => {
            g.path(&[(4.0, 9.0), (8.0, 9.0), (13.0, 5.0), (13.0, 19.0), (8.0, 15.0), (4.0, 15.0), (4.0, 9.0)]);
            g.path(&[(16.0, 9.0), (18.0, 12.0), (16.0, 15.0)]);
            g.path(&[(19.0, 7.0), (21.5, 12.0), (19.0, 17.0)]);
        }
        Icon::Microphone => {
            g.rounded(9.0, 3.0, 6.0, 12.0, 3.0);
            g.path(&[(5.0, 11.0), (5.0, 13.0)]);
            g.path(&[(19.0, 11.0), (19.0, 13.0)]);
            g.path(&[(5.0, 13.0), (7.0, 16.0), (12.0, 17.0), (17.0, 16.0), (19.0, 13.0)]);
            g.path(&[(12.0, 17.0), (12.0, 21.0)]);
            g.path(&[(8.0, 21.0), (16.0, 21.0)]);
        }
    }
}

struct Glyph<'a, 'b> {
    p: &'a mut Painter<'b>,
    r: R,
    color: u32,
    width: f32,
    rotation: f32,
}
impl Glyph<'_, '_> {
    fn path(&mut self, points: &[(f32, f32)]) {
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let points: Vec<_> = points
            .iter()
            .map(|&(x, y)| {
                let (x, y) = (x - 12.0, y - 12.0);
                (
                    self.r.cx() + (x * cos - y * sin) * self.r.w / 24.0,
                    self.r.cy() + (x * sin + y * cos) * self.r.h / 24.0,
                )
            })
            .collect();
        self.p
            .stroke_path(&points, self.color, self.width * self.r.w / 24.0);
    }
    fn circle(&mut self, x: f32, y: f32, radius: f32) {
        let mut points = Vec::new();
        arc(&mut points, (x, y), radius, 0.0, 360.0);
        self.path(&points);
    }
    fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let mut points = Vec::new();
        for (center, start) in [
            ((x + radius, y + radius), 180.0),
            ((x + w - radius, y + radius), 270.0),
            ((x + w - radius, y + h - radius), 0.0),
            ((x + radius, y + h - radius), 90.0),
        ] {
            arc(&mut points, center, radius, start, 90.0);
        }
        points.push(points[0]);
        self.path(&points);
    }
}
fn arc(points: &mut Vec<(f32, f32)>, center: (f32, f32), radius: f32, start: f32, sweep: f32) {
    let steps = (sweep.abs() / 5.0).ceil().max(1.0) as usize;
    for i in 0..=steps {
        let a = (start + sweep * i as f32 / steps as f32).to_radians();
        points.push((center.0 + radius * a.cos(), center.1 + radius * a.sin()));
    }
}
fn cubic(points: &mut Vec<(f32, f32)>, a: (f32, f32), b: (f32, f32), end: (f32, f32)) {
    let start = *points.last().unwrap();
    for i in 1..=24 {
        let t = i as f32 / 24.0;
        let u = 1.0 - t;
        points.push((
            u * u * u * start.0 + 3.0 * u * u * t * a.0 + 3.0 * u * t * t * b.0 + t * t * t * end.0,
            u * u * u * start.1 + 3.0 * u * u * t * a.1 + 3.0 * u * t * t * b.1 + t * t * t * end.1,
        ));
    }
}
