//! Native taskbar text at its measured size, without stretching glyph outlines.

use super::StripItem;
use crate::{tray_prefs::TrayKind, win32::Handle};
use std::{mem, ptr};
use winapi::{shared::windef::{HDC, HGDIOBJ, SIZE}, um::wingdi::*};

struct Font(HGDIOBJ);
impl Drop for Font {
    fn drop(&mut self) { unsafe { DeleteObject(self.0); } }
}

struct Dc(HDC);
impl Dc {
    fn new() -> Option<Self> {
        let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
        (!dc.is_null()).then_some(Self(dc))
    }
}
impl Drop for Dc {
    fn drop(&mut self) { unsafe { DeleteDC(self.0); } }
}

struct Selection(HDC, HGDIOBJ);
impl Selection {
    fn new(dc: HDC, object: HGDIOBJ) -> Option<Self> {
        let old = unsafe { SelectObject(dc, object) };
        (!old.is_null() && old != HGDI_ERROR).then_some(Self(dc, old))
    }
}
impl Drop for Selection {
    fn drop(&mut self) { unsafe { SelectObject(self.0, self.1); } }
}

struct Dib { handle: HGDIOBJ, pixels: *mut u32 }
impl Dib {
    fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 { return None; }
        let mut info: BITMAPINFO = unsafe { mem::zeroed() };
        info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = ptr::null_mut();
        let bitmap = unsafe { CreateDIBSection(ptr::null_mut(), &info, DIB_RGB_COLORS,
            &mut bits, ptr::null_mut(), 0) };
        if bitmap.is_null() { return None; }
        let dib = Self { handle: bitmap as _, pixels: bits.cast() };
        (!bits.is_null()).then_some(dib)
    }
}
impl Drop for Dib {
    fn drop(&mut self) { unsafe { DeleteObject(self.handle); } }
}

fn px(value: f32, scale: f32) -> i32 { (value * scale).round().max(1.0) as i32 }

fn native_font(candidates: &[(&str, f32)], scale: f32) -> Option<Font> {
    let dc = Dc::new()?;
    for (face, size) in candidates {
        let name = crate::win32::to_wide(face);
        let font = unsafe { CreateFontW(-px(*size, scale), 0, 0, 0, FW_NORMAL,
            0, 0, 0, DEFAULT_CHARSET as _, OUT_DEFAULT_PRECIS as _, CLIP_DEFAULT_PRECIS as _,
            ANTIALIASED_QUALITY as _, DEFAULT_PITCH as _, name.as_ptr()) };
        if font.is_null() { continue; }
        let font = Font(font as _);
        let _selection = Selection::new(dc.0, font.0)?;
        let mut actual = [0u16; 128];
        let length = unsafe { GetTextFaceW(dc.0, actual.len() as i32, actual.as_mut_ptr()) };
        let length = (length.max(0) as usize).min(actual.len());
        let end = actual[..length].iter().position(|c| *c == 0).unwrap_or(length);
        if String::from_utf16_lossy(&actual[..end]).eq_ignore_ascii_case(face) {
            return Some(font);
        }
    }
    None
}

struct Mask { width: i32, height: i32, alpha: Vec<u8> }
impl Mask {
    fn text(text: &str, font: &Font, light: bool) -> Option<Self> {
        let dc = Dc::new()?;
        let _font = Selection::new(dc.0, font.0)?;
        let text: Vec<_> = text.encode_utf16().collect();
        let mut size: SIZE = unsafe { mem::zeroed() };
        if unsafe { GetTextExtentPoint32W(dc.0, text.as_ptr(), text.len() as i32, &mut size) } == 0 {
            return None;
        }
        let (width, height) = (size.cx + 8, size.cy + 8);
        let dib = Dib::new(width, height)?;
        let _bitmap = Selection::new(dc.0, dib.handle)?;
        let pixels = unsafe { std::slice::from_raw_parts_mut(dib.pixels, (width * height) as usize) };
        pixels.fill(if light { 0x00ff_ffff } else { 0 });
        unsafe {
            SetBkMode(dc.0, TRANSPARENT as _);
            SetTextColor(dc.0, if light { 0 } else { 0x00ff_ffff });
            if TextOutW(dc.0, 4, 4, text.as_ptr(), text.len() as i32) == 0 { return None; }
            GdiFlush();
        }
        let coverage = |pixel: u32| {
            let shade = ((pixel & 255) + ((pixel >> 8) & 255) + ((pixel >> 16) & 255)) / 3;
            if light { (255 - shade) as u8 } else { shade as u8 }
        };
        let (mut left, mut top, mut right, mut bottom) = (width, height, -1, -1);
        for y in 0..height {
            for x in 0..width {
                if coverage(pixels[(y * width + x) as usize]) != 0 {
                    left = left.min(x); right = right.max(x);
                    top = top.min(y); bottom = bottom.max(y);
                }
            }
        }
        if right < left { return None; }
        let mut alpha = Vec::with_capacity(((right - left + 1) * (bottom - top + 1)) as usize);
        for y in top..=bottom {
            for x in left..=right { alpha.push(coverage(pixels[(y * width + x) as usize])); }
        }
        Some(Self { width: right - left + 1, height: bottom - top + 1, alpha })
    }
}

fn glyph(kind: TrayKind) -> &'static str {
    match kind {
        TrayKind::Mouse => "\u{e962}",
        TrayKind::Keyboard => "\u{e765}",
        TrayKind::Headset => "\u{e7f6}",
        TrayKind::Gamepad => "\u{e7fc}",
        TrayKind::Auto | TrayKind::Generic => "\u{e772}",
    }
}

struct Item { icon: Mask, label: Mask, charge: Option<Mask>, color: u32 }

pub(super) struct Layout {
    items: Vec<Item>,
    foreground: u32,
    charge_color: u32,
    padding: i32,
    icon_gap: i32,
    device_gap: i32,
    charge_gap: i32,
}

impl Layout {
    pub fn new(items: &[StripItem], light: bool, scale: f32) -> Option<Self> {
        if items.is_empty() || !scale.is_finite() || scale <= 0.0 { return None; }
        let text = native_font(&[("Segoe UI Variable Text", 13.0), ("Segoe UI", 12.0)], scale)?;
        let icons = native_font(&[("Segoe Fluent Icons", 16.0), ("Segoe MDL2 Assets", 16.0)], scale)?;
        let marks = native_font(&[("Segoe Fluent Icons", 11.0), ("Segoe MDL2 Assets", 11.0)], scale)?;
        let foreground = if light { 0xff25_302b } else { 0xffe1_e8df };
        let red = if light { 0xffc4_2b1c } else { 0xffff_6b6b };
        let items = items.iter().map(|item| Some(Item {
            icon: Mask::text(glyph(item.kind), &icons, light)?,
            label: Mask::text(&item.level.map(|v| v.label(true)).unwrap_or_else(|| "—".into()), &text, light)?,
            charge: if item.charging && item.level.is_some() {
                Some(Mask::text("\u{e945}", &marks, light)?)
            } else { None },
            color: if item.level.is_some_and(|v| v.value <= 20) { red } else { foreground },
        })).collect::<Option<Vec<_>>>()?;
        Some(Self { items, foreground,
            charge_color: if light { 0xff50_7b3b } else { 0xffa1_c486 },
            padding: px(6.0, scale), icon_gap: px(4.0, scale),
            device_gap: px(10.0, scale), charge_gap: px(2.0, scale),
        })
    }

    pub fn width(&self, count: usize) -> i32 {
        let count = count.min(self.items.len());
        if count == 0 { return 0; }
        self.padding * 2 + self.items[..count].iter().map(|item| {
            item.icon.width + self.icon_gap + item.label.width
                + item.charge.as_ref().map_or(0, |mark| self.charge_gap + mark.width)
        }).sum::<i32>() + self.device_gap * (count - 1) as i32
    }

    pub fn visible_count(&self, available: i32) -> usize {
        (1..=self.items.len()).rev().find(|count| self.width(*count) <= available).unwrap_or(0)
    }

    pub fn raster(&self, count: usize, height: i32) -> Option<Raster> {
        let count = count.min(self.items.len());
        let width = self.width(count);
        if width <= 0 || height <= 0 { return None; }
        let mut raster = Raster { width, height, pixels: vec![0x0100_0000; (width * height) as usize] };
        let mut x = self.padding;
        for item in &self.items[..count] {
            raster.mask(&item.icon, x, self.foreground);
            x += item.icon.width + self.icon_gap;
            raster.mask(&item.label, x, item.color);
            x += item.label.width;
            if let Some(mark) = &item.charge {
                x += self.charge_gap;
                raster.mask(mark, x, self.charge_color);
                x += mark.width;
            }
            x += self.device_gap;
        }
        Some(raster)
    }
}

pub(super) struct Raster { pub width: i32, pub height: i32, pub pixels: Vec<u32> }
impl Raster {
    fn mask(&mut self, mask: &Mask, left: i32, color: u32) {
        let top = (self.height - mask.height) / 2;
        for y in 0..mask.height {
            for x in 0..mask.width {
                let (dx, dy) = (left + x, top + y);
                if dx < 0 || dx >= self.width || dy < 0 || dy >= self.height { continue; }
                let alpha = mask.alpha[(y * mask.width + x) as usize] as u32;
                if alpha == 0 { continue; }
                let channel = |shift: u32| ((color >> shift) & 255u32) * alpha / 255;
                let a = alpha + (255 - alpha) / 255;
                self.pixels[(dy * self.width + dx) as usize] =
                    (a << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0);
            }
        }
    }

    pub fn bitmap(&self) -> Option<Handle> {
        let dib = Dib::new(self.width, self.height)?;
        unsafe { ptr::copy_nonoverlapping(self.pixels.as_ptr(), dib.pixels, self.pixels.len()); }
        let handle = dib.handle as Handle;
        mem::forget(dib);
        Some(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(value: u8, kind: TrayKind) -> StripItem {
        StripItem { level: Some(crate::device::BatteryPercent { value, approximate: false }), charging: false, kind }
    }

    #[test]
    fn native_layout_fits_whole_devices_and_single_device_keeps_its_icon() {
        let items = [item(75, TrayKind::Mouse), item(100, TrayKind::Keyboard),
            item(18, TrayKind::Headset), item(67, TrayKind::Gamepad)];
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let layout = Layout::new(&items, true, scale).expect("native fonts");
            let one = Layout::new(&items[..1], true, scale).unwrap();
            assert_eq!(one.width(1), layout.width(1));
            assert!(one.items[0].icon.width > 0);
            assert!(layout.width(2) < (130.0 * scale) as i32);
            assert!(layout.width(4) < (260.0 * scale) as i32);
            for count in 1..=items.len() {
                assert_eq!(layout.visible_count(layout.width(count)), count);
                assert_eq!(layout.visible_count(layout.width(count) - 1), count - 1);
            }
            assert_eq!(layout.visible_count(0), 0);
        }
    }

    #[test]
    fn native_font_rasters_keep_size_alpha_and_center_at_each_taskbar_scale() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for light in [true, false] {
                for value in [5, 11, 75, 100] {
                    let mut data = item(value, TrayKind::Mouse);
                    data.charging = value == 100;
                    data.level.as_mut().unwrap().approximate = value == 5;
                    let layout = Layout::new(&[data], light, scale).unwrap();
                    let label = &layout.items[0].label;
                    assert!(label.height <= px(11.0, scale) + 1);
                    assert!(label.height >= px(7.0, scale));
                    let height = px(40.0, scale);
                    let raster = layout.raster(1, height).unwrap();
                    let first = layout.padding + layout.items[0].icon.width + layout.icon_gap;
                    let rows: Vec<_> = (0..height).filter(|y| (first..first + label.width)
                        .any(|x| raster.pixels[(y * raster.width + x) as usize] >> 24 > 32)).collect();
                    let center = (rows.first().unwrap() + rows.last().unwrap() + 1) as f32 / 2.0;
                    assert!((center - height as f32 / 2.0).abs() <= 1.0);
                    for pixel in &raster.pixels {
                        assert!([0, 8, 16].into_iter().all(|s| (pixel >> s) & 255 <= pixel >> 24));
                    }
                    // Empty space is a hit target too; clicks must not fall through to Explorer.
                    assert!(raster.pixels.iter().all(|pixel| pixel >> 24 != 0));
                    let bitmap = raster.bitmap().expect("native DIB");
                    unsafe { crate::win32::DeleteObject(bitmap); }
                }
            }
        }
    }
}
