//! The tray icon with an amber dot in its lower right corner, for when something needs the user: a
//! link that is down, or setup waiting. It is drawn from Koppel's own icon at run time, so no second
//! icon ships. Pixels are 32-bit with straight alpha, as Windows reads an icon's colour bitmap.

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
    DeleteObject, GetDC, GetDIBits, HBITMAP, HGDIOBJ, ReleaseDC,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, GetIconInfo, HICON, ICONINFO};

/// The design system's warning amber, #F59E0B, and white for the ring that keeps it apart from the
/// icon.
const AMBER: u32 = 0xFFF5_9E0B;
const WHITE: u32 = 0xFFFF_FFFF;

/// The attention icon at `size` pixels square, or `None` when it cannot be made; the plain icon is
/// used then.
pub fn create(icon: HICON, size: i32) -> Option<HICON> {
    if icon.is_invalid() || size <= 0 {
        return None;
    }
    let mut info = ICONINFO::default();
    // SAFETY: the icon is valid and the out parameter ours; its bitmaps are deleted below.
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let made = with_bitmaps(&info, size);
    // SAFETY: GetIconInfo made copies of these for us to delete.
    unsafe {
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
    }
    made
}

fn with_bitmaps(info: &ICONINFO, size: i32) -> Option<HICON> {
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let side = size as usize;
    let mut pixels = vec![0u32; side * side];
    // SAFETY: the screen DC is released, and every bitmap made here deleted, before returning; the
    // pixel buffers are as large as the header says.
    unsafe {
        let screen = GetDC(None);
        let read = GetDIBits(
            screen,
            info.hbmColor,
            0,
            size as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        if read != size {
            ReleaseDC(None, screen);
            return None;
        }
        // An icon without alpha (all zero) is opaque wherever it has colour.
        if pixels.iter().all(|p| p >> 24 == 0) {
            for p in &mut pixels {
                *p |= 0xFF00_0000;
            }
        }
        draw_dot(&mut pixels, side);

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(Some(screen), &header, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, screen);
        let color: HBITMAP = color.ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), pixels.len());

        // With a colour bitmap that has alpha, Windows draws from the alpha and the mask only has
        // to exist.
        let mask_bits = vec![0u8; side.div_ceil(16) * 2 * side];
        let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
        let attention = ICONINFO {
            fIcon: true.into(),
            hbmMask: mask,
            hbmColor: color,
            ..Default::default()
        };
        let icon = CreateIconIndirect(&attention).ok();
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon
    }
}

/// A filled amber circle with a white ring, in the lower right, about 45% of the icon across, with
/// soft edges.
pub fn draw_dot(pixels: &mut [u32], size: usize) {
    let s = size as f64;
    let outer = s * 0.225;
    let ring = (s / 16.0).max(1.0);
    let centre = s - outer - 0.25;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - centre;
            let dy = y as f64 + 0.5 - centre;
            let distance = (dx * dx + dy * dy).sqrt();
            let ring_cover = (outer + 0.5 - distance).clamp(0.0, 1.0);
            if ring_cover <= 0.0 {
                continue;
            }
            let fill_cover = (outer - ring + 0.5 - distance).clamp(0.0, 1.0);
            let i = y * size + x;
            pixels[i] = blend(blend(pixels[i], WHITE, ring_cover), AMBER, fill_cover);
        }
    }
}

/// Lays `top` (opaque) over `bottom` at `cover`, in straight alpha.
fn blend(bottom: u32, top: u32, cover: f64) -> u32 {
    if cover <= 0.0 {
        return bottom;
    }
    let bottom_alpha = f64::from(bottom >> 24) / 255.0;
    let alpha = cover + bottom_alpha * (1.0 - cover);
    let channel = |shift: u32| {
        let b = f64::from((bottom >> shift) & 0xFF) * bottom_alpha * (1.0 - cover);
        let t = f64::from((top >> shift) & 0xFF) * cover;
        ((b + t) / alpha).round() as u32
    };
    (((alpha * 255.0).round() as u32) << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_is_amber_in_the_lower_right_and_leaves_the_rest() {
        let size = 32;
        let mut pixels = vec![0xFF00_0000u32; size * size];
        draw_dot(&mut pixels, size);
        let centre = (size as f64 - size as f64 * 0.225 - 0.25) as usize;
        assert_eq!(pixels[centre * size + centre], AMBER);
        assert_eq!(pixels[0], 0xFF00_0000);
        assert_eq!(pixels[size - 1], 0xFF00_0000);
        // The ring is white where it meets the icon.
        let edge = centre - (size as f64 * 0.225) as usize + 1;
        assert_eq!(pixels[centre * size + edge] & 0x00FF_FFFF, 0x00FF_FFFF);
    }

    #[test]
    fn blending_onto_transparency_keeps_the_top_colour() {
        assert_eq!(blend(0, AMBER, 1.0), AMBER);
        assert_eq!(blend(0x1234_5678, AMBER, 0.0), 0x1234_5678);
        let half = blend(0, WHITE, 0.5);
        assert_eq!(half >> 24, 128);
        assert_eq!(half & 0x00FF_FFFF, 0x00FF_FFFF);
    }
}
