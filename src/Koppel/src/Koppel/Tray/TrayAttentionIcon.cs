using System.Runtime.InteropServices;
using System.Runtime.Versioning;

namespace Koppel.Tray;

/// <summary>
/// The tray icon with an amber dot in its lower right corner, for when something needs the user: a link that is down, or
/// setup waiting. It is drawn from Koppel's own icon at run time, so no second icon ships. Pixels are 32-bit with straight
/// alpha, as Windows reads an icon's colour bitmap.
/// </summary>
[SupportedOSPlatform("windows")]
internal static unsafe partial class TrayAttentionIcon
{
    /// <summary>The attention icon at <paramref name="size"/> pixels square, or 0 when it cannot be made; the plain icon is used then.</summary>
    public static nint Create(nint icon, int size)
    {
        if (icon == 0 || size <= 0 || !GetIconInfo(icon, out var info))
        {
            return 0;
        }

        nint screen = 0;
        nint color = 0;
        nint mask = 0;
        try
        {
            var header = new BITMAPINFOHEADER
            {
                biSize = (uint)sizeof(BITMAPINFOHEADER),
                biWidth = size,
                biHeight = -size, // top-down
                biPlanes = 1,
                biBitCount = 32,
            };

            var pixels = new uint[size * size];
            screen = GetDC(0);
            fixed (uint* bits = pixels)
            {
                if (GetDIBits(screen, info.hbmColor, 0, (uint)size, bits, &header, 0) != size)
                {
                    return 0;
                }
            }

            // An icon without alpha (all zero) is opaque wherever it has colour.
            if (pixels.All(p => p >> 24 == 0))
            {
                for (var i = 0; i < pixels.Length; i++)
                {
                    pixels[i] |= 0xFF000000;
                }
            }

            TrayAttentionDot.Draw(pixels, size);

            color = CreateDIBSection(screen, &header, 0, out var target, 0, 0);
            if (color == 0)
            {
                return 0;
            }

            fixed (uint* source = pixels)
            {
                Buffer.MemoryCopy(source, (void*)target, pixels.Length * 4L, pixels.Length * 4L);
            }

            // With a colour bitmap that has alpha, Windows draws from the alpha and the mask only has to exist.
            var maskBits = new byte[((size + 15) / 16) * 2 * size];
            fixed (byte* maskData = maskBits)
            {
                mask = CreateBitmap(size, size, 1, 1, maskData);
            }

            var attention = new ICONINFO { fIcon = 1, hbmMask = mask, hbmColor = color };
            return CreateIconIndirect(&attention);
        }
        finally
        {
            if (screen != 0)
            {
                _ = ReleaseDC(0, screen);
            }

            DeleteObject(color);
            DeleteObject(mask);
            DeleteObject(info.hbmColor);
            DeleteObject(info.hbmMask);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ICONINFO
    {
        public int fIcon;
        public uint xHotspot;
        public uint yHotspot;
        public nint hbmMask;
        public nint hbmColor;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAPINFOHEADER
    {
        public uint biSize;
        public int biWidth;
        public int biHeight;
        public ushort biPlanes;
        public ushort biBitCount;
        public uint biCompression;
        public uint biSizeImage;
        public int biXPelsPerMeter;
        public int biYPelsPerMeter;
        public uint biClrUsed;
        public uint biClrImportant;
    }

    [LibraryImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool GetIconInfo(nint icon, out ICONINFO info);

    [LibraryImport("user32.dll")]
    private static partial nint CreateIconIndirect(ICONINFO* info);

    [LibraryImport("user32.dll")]
    private static partial nint GetDC(nint window);

    [LibraryImport("user32.dll")]
    private static partial int ReleaseDC(nint window, nint dc);

    [LibraryImport("gdi32.dll")]
    private static partial int GetDIBits(nint dc, nint bitmap, uint start, uint lines, void* bits, BITMAPINFOHEADER* info, uint usage);

    [LibraryImport("gdi32.dll")]
    private static partial nint CreateDIBSection(nint dc, BITMAPINFOHEADER* info, uint usage, out nint bits, nint section, uint offset);

    [LibraryImport("gdi32.dll")]
    private static partial nint CreateBitmap(int width, int height, uint planes, uint bitCount, void* bits);

    [LibraryImport("gdi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static partial bool DeleteObject(nint handle);
}

/// <summary>The dot itself, kept apart from Win32 so it is tested on every platform.</summary>
internal static class TrayAttentionDot
{
    // The design system's warning amber, #F59E0B, and white for the ring that keeps it apart from the icon.
    private const uint Amber = 0xFFF59E0B;
    private const uint White = 0xFFFFFFFF;

    /// <summary>A filled amber circle with a white ring, in the lower right, about 45% of the icon across, with soft edges.</summary>
    public static void Draw(uint[] pixels, int size)
    {
        var outer = size * 0.225;
        var ring = Math.Max(1.0, size / 16.0);
        var centre = size - outer - 0.25;
        for (var y = 0; y < size; y++)
        {
            for (var x = 0; x < size; x++)
            {
                var distance = Math.Sqrt(Math.Pow(x + 0.5 - centre, 2) + Math.Pow(y + 0.5 - centre, 2));
                var ringCover = Math.Clamp(outer + 0.5 - distance, 0, 1);
                if (ringCover <= 0)
                {
                    continue;
                }

                var fillCover = Math.Clamp(outer - ring + 0.5 - distance, 0, 1);
                var i = (y * size) + x;
                pixels[i] = Blend(Blend(pixels[i], White, ringCover), Amber, fillCover);
            }
        }
    }

    /// <summary>Lays <paramref name="top"/> (opaque) over <paramref name="bottom"/> at <paramref name="cover"/>, in straight alpha.</summary>
    private static uint Blend(uint bottom, uint top, double cover)
    {
        if (cover <= 0)
        {
            return bottom;
        }

        var bottomAlpha = (bottom >> 24) / 255.0;
        var alpha = cover + (bottomAlpha * (1 - cover));
        uint Channel(int shift)
        {
            var b = ((bottom >> shift) & 0xFF) * bottomAlpha * (1 - cover);
            var t = ((top >> shift) & 0xFF) * cover;
            return (uint)Math.Round((b + t) / alpha);
        }

        return ((uint)Math.Round(alpha * 255) << 24) | (Channel(16) << 16) | (Channel(8) << 8) | Channel(0);
    }
}
