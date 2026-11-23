//! Feedback and About icons drawn on a shared 24×24 coordinate grid.
use super::gfx::{Painter, R};

#[derive(Clone, Copy)]
pub enum Icon {
    Message,
    History,
    File,
    Globe,
    Send,
    Arrow,
    Image,
    Shield,
    Plus,
    Next,
    Close,
    Chevron,
    Check,
    Copy,
    Info,
    Mouse,
    Keyboard,
    Headset,
}

pub fn draw(p: &mut Painter, icon: Icon, r: R, color: u32) {
    let mut g = Glyph { p, r, color };
    match icon {
        Icon::Message => {
            let mut path = vec![(20.0, 11.5)];
            arc(&mut path, (12.5, 11.5), 7.5, 0.0, 90.0);
            path.extend([(6.0, 19.0), (2.0, 22.0), (2.0, 11.5)]);
            arc(&mut path, (9.5, 11.5), 7.5, 180.0, 90.0);
            path.push((12.5, 4.0));
            arc(&mut path, (12.5, 11.5), 7.5, 270.0, 90.0);
            g.path(&path);
            g.path(&[(7.0, 10.0), (15.0, 10.0)]);
            g.path(&[(7.0, 14.0), (12.0, 14.0)]);
        }
        Icon::History => {
            let mut path = vec![(3.0, 10.0)];
            svg_arc(&mut path, (5.7, 18.5), 9.0, true, true);
            g.path(&path);
            g.path(&[(3.0, 4.0), (3.0, 10.0), (9.0, 10.0)]);
            g.path(&[(12.0, 7.0), (12.0, 12.0), (15.0, 14.0)]);
        }
        Icon::File => {
            let mut path = vec![(14.0, 2.0), (6.0, 2.0)];
            arc(&mut path, (6.0, 4.0), 2.0, 270.0, -90.0);
            path.push((4.0, 20.0));
            arc(&mut path, (6.0, 20.0), 2.0, 180.0, -90.0);
            path.push((18.0, 22.0));
            arc(&mut path, (18.0, 20.0), 2.0, 90.0, -90.0);
            path.extend([(20.0, 8.0), (14.0, 2.0)]);
            g.path(&path);
            g.path(&[(14.0, 2.0), (14.0, 8.0), (20.0, 8.0)]);
            g.path(&[(8.0, 12.0), (16.0, 12.0)]);
            g.path(&[(8.0, 16.0), (13.0, 16.0)]);
        }
        Icon::Globe => {
            g.ellipse(12.0, 12.0, 9.0, 9.0);
            g.ellipse(12.0, 12.0, 4.0, 9.0);
            g.path(&[(3.0, 12.0), (21.0, 12.0)]);
        }
        Icon::Send => {
            g.path(&[
                (21.0, 3.0),
                (14.0, 21.0),
                (10.0, 14.0),
                (3.0, 10.0),
                (21.0, 3.0),
            ]);
            g.path(&[(10.0, 14.0), (16.0, 8.0)]);
        }
        Icon::Arrow => {
            g.path(&[(9.0, 5.0), (2.0, 12.0), (9.0, 19.0)]);
            g.path(&[(2.0, 12.0), (21.0, 12.0)]);
        }
        Icon::Image => {
            g.rounded(3.0, 3.0, 18.0, 18.0, 3.0);
            g.ellipse(8.0, 8.0, 1.5, 1.5);
            g.path(&[(21.0, 15.0), (15.0, 9.0), (3.0, 21.0)]);
        }
        Icon::Shield => {
            let mut path = vec![(12.0, 2.0), (20.0, 5.0), (20.0, 11.0)];
            cubic(&mut path, (20.0, 16.0), (15.5, 19.5), (12.0, 22.0));
            cubic(&mut path, (8.5, 19.5), (4.0, 16.0), (4.0, 11.0));
            path.extend([(4.0, 5.0), (12.0, 2.0)]);
            g.path(&path);
            g.path(&[(8.0, 11.0), (11.0, 14.0), (16.0, 9.0)]);
        }
        Icon::Plus => {
            g.path(&[(12.0, 5.0), (12.0, 19.0)]);
            g.path(&[(5.0, 12.0), (19.0, 12.0)]);
        }
        Icon::Next => g.path(&[(9.0, 5.0), (16.0, 12.0), (9.0, 19.0)]),
        Icon::Close => {
            g.path(&[(6.0, 6.0), (18.0, 18.0)]);
            g.path(&[(6.0, 18.0), (18.0, 6.0)]);
        }
        Icon::Chevron => g.path(&[(8.0, 10.0), (12.0, 14.0), (16.0, 10.0)]),
        Icon::Check => g.path(&[(5.0, 12.0), (9.0, 16.0), (19.0, 6.0)]),
        Icon::Copy => {
            g.rounded(8.0, 8.0, 12.0, 13.0, 2.0);
            let mut path = vec![(16.0, 8.0), (16.0, 4.0)];
            arc(&mut path, (15.0, 4.0), 1.0, 0.0, -90.0);
            path.push((4.0, 3.0));
            arc(&mut path, (4.0, 4.0), 1.0, 270.0, -90.0);
            path.push((3.0, 15.0));
            arc(&mut path, (4.0, 15.0), 1.0, 180.0, -90.0);
            path.push((8.0, 16.0));
            g.path(&path);
        }
        Icon::Info => {
            g.ellipse(12.0, 12.0, 9.0, 9.0);
            g.path(&[(12.0, 11.0), (12.0, 17.0)]);
            g.path(&[(12.0, 7.0), (12.01, 7.0)]);
        }
        Icon::Mouse => {
            g.rounded(6.0, 2.0, 12.0, 20.0, 6.0);
            g.path(&[(12.0, 2.0), (12.0, 9.0)]);
            g.path(&[(6.0, 9.0), (18.0, 9.0)]);
        }
        Icon::Keyboard => {
            g.rounded(2.0, 5.0, 20.0, 14.0, 3.0);
            for y in [9.0, 12.0] {
                for x in [6.0, 10.0, 14.0, 18.0] {
                    g.path(&[(x, y), (x + 0.01, y)]);
                }
            }
            g.path(&[(7.0, 15.0), (17.0, 15.0)]);
        }
        Icon::Headset => {
            let mut path = vec![(4.0, 14.0), (4.0, 11.0)];
            arc(&mut path, (12.0, 11.0), 8.0, 180.0, 180.0);
            path.push((20.0, 14.0));
            g.path(&path);
            g.rounded(3.0, 12.0, 4.0, 8.0, 2.0);
            g.rounded(17.0, 12.0, 4.0, 8.0, 2.0);
        }
    }
}

struct Glyph<'a, 'b> {
    p: &'a mut Painter<'b>,
    r: R,
    color: u32,
}

impl Glyph<'_, '_> {
    fn path(&mut self, points: &[(f32, f32)]) {
        let scaled: Vec<_> = points
            .iter()
            .map(|&(x, y)| {
                (
                    self.r.x + x * self.r.w / 24.0,
                    self.r.y + y * self.r.h / 24.0,
                )
            })
            .collect();
        self.p
            .stroke_path(&scaled, self.color, 1.75 * self.r.w.min(self.r.h) / 24.0);
    }

    fn ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32) {
        let points: Vec<_> = (0..=72)
            .map(|index| {
                let angle = (index as f32 * 5.0).to_radians();
                (x + rx * angle.cos(), y + ry * angle.sin())
            })
            .collect();
        self.path(&points);
    }

    fn rounded(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        let mut path = Vec::new();
        for (center, start) in [
            ((x + radius, y + radius), 180.0),
            ((x + w - radius, y + radius), 270.0),
            ((x + w - radius, y + h - radius), 0.0),
            ((x + radius, y + h - radius), 90.0),
        ] {
            arc(&mut path, center, radius, start, 90.0);
        }
        path.push(path[0]);
        self.path(&path);
    }
}

fn arc(points: &mut Vec<(f32, f32)>, center: (f32, f32), radius: f32, start: f32, sweep: f32) {
    let steps = (sweep.abs() / 5.0).ceil().max(1.0) as usize;
    for index in 0..=steps {
        let angle = (start + sweep * index as f32 / steps as f32).to_radians();
        points.push((
            center.0 + radius * angle.cos(),
            center.1 + radius * angle.sin(),
        ));
    }
}

fn svg_arc(points: &mut Vec<(f32, f32)>, end: (f32, f32), radius: f32, large: bool, sweep: bool) {
    let start = *points.last().unwrap();
    let (x, y) = ((start.0 - end.0) / 2.0, (start.1 - end.1) / 2.0);
    let distance = x * x + y * y;
    let factor = ((radius * radius - distance).max(0.0) / distance).sqrt()
        * if large == sweep { -1.0 } else { 1.0 };
    let center = (
        (start.0 + end.0) / 2.0 + factor * y,
        (start.1 + end.1) / 2.0 - factor * x,
    );
    let angle = (start.1 - center.1).atan2(start.0 - center.0).to_degrees();
    let mut delta = (end.1 - center.1).atan2(end.0 - center.0).to_degrees() - angle;
    if sweep && delta < 0.0 {
        delta += 360.0;
    }
    if !sweep && delta > 0.0 {
        delta -= 360.0;
    }
    arc(points, center, radius, angle, delta);
}

fn cubic(points: &mut Vec<(f32, f32)>, a: (f32, f32), b: (f32, f32), end: (f32, f32)) {
    let start = *points.last().unwrap();
    for index in 1..=24 {
        let t = index as f32 / 24.0;
        let u = 1.0 - t;
        points.push((
            u.powi(3) * start.0 + 3.0 * u * u * t * a.0 + 3.0 * u * t * t * b.0 + t.powi(3) * end.0,
            u.powi(3) * start.1 + 3.0 * u * u * t * a.1 + 3.0 * u * t * t * b.1 + t.powi(3) * end.1,
        ));
    }
}
