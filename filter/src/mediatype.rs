//! AM_MEDIA_TYPE helpers.

use windows::Win32::Graphics::Gdi::BITMAPINFOHEADER;
use windows::Win32::Media::MediaFoundation::{
    AM_MEDIA_TYPE, FORMAT_VideoInfo, FORMAT_VideoInfo2, MEDIASUBTYPE_ARGB32, MEDIASUBTYPE_RGB24,
    MEDIASUBTYPE_RGB32, MEDIASUBTYPE_RGB555, MEDIASUBTYPE_RGB565, MEDIASUBTYPE_RGB8,
    MEDIATYPE_Video, VIDEOINFOHEADER, VIDEOINFOHEADER2,
};
use windows::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};

use crate::state::FormatInfo;

// MMDxShow's 32-bit RGB output subtype (the classic "DIB Sequential" filter
// MMD ships, which pushes MMD's render surface as a 32bpp DIB).
pub const MEDIASUBTYPE_MMDXSHOW_RGB32: windows::core::GUID =
    windows::core::GUID::from_u128(0x773C9AC0_3274_11D0_B724_00AA006C1A01);

pub fn check_input(mt: &AM_MEDIA_TYPE) -> bool {
    if mt.majortype != MEDIATYPE_Video {
        return false;
    }
    let sub = mt.subtype;
    if !(sub == MEDIASUBTYPE_RGB24
        || sub == MEDIASUBTYPE_RGB32
        || sub == MEDIASUBTYPE_ARGB32
        || sub == MEDIASUBTYPE_RGB565
        || sub == MEDIASUBTYPE_RGB555
        || sub == MEDIASUBTYPE_RGB8
        || sub == MEDIASUBTYPE_MMDXSHOW_RGB32)
    {
        return false;
    }
    mt.formattype == FORMAT_VideoInfo || mt.formattype == FORMAT_VideoInfo2
}

pub fn parse_input(mt: &AM_MEDIA_TYPE) -> Option<FormatInfo> {
    if !check_input(mt) {
        return None;
    }
    let (width, height, bottom_up, frame_dur) = unsafe {
        if mt.formattype == FORMAT_VideoInfo {
            let vih = &*(mt.pbFormat as *const VIDEOINFOHEADER);
            let h = vih.bmiHeader.biHeight;
            (
                vih.bmiHeader.biWidth,
                h.abs(),
                h > 0,
                if vih.AvgTimePerFrame > 0 {
                    vih.AvgTimePerFrame
                } else {
                    1
                },
            )
        } else if mt.formattype == FORMAT_VideoInfo2 {
            let vih = &*(mt.pbFormat as *const VIDEOINFOHEADER2);
            let h = vih.bmiHeader.biHeight;
            (
                vih.bmiHeader.biWidth,
                h.abs(),
                h > 0,
                if vih.AvgTimePerFrame > 0 {
                    vih.AvgTimePerFrame
                } else {
                    1
                },
            )
        } else {
            return None;
        }
    };

    let pix_fmt = match mt.subtype {
        // DirectShow RGB24 samples are stored in DIB order: B,G,R.
        s if s == MEDIASUBTYPE_RGB24 => "bgr24".to_string(),
        s if s == MEDIASUBTYPE_RGB32 || s == MEDIASUBTYPE_ARGB32 => "bgra".to_string(),
        s if s == MEDIASUBTYPE_MMDXSHOW_RGB32 => "bgra".to_string(),
        s if s == MEDIASUBTYPE_RGB565 => "rgb565le".to_string(),
        s if s == MEDIASUBTYPE_RGB555 => "rgb555le".to_string(),
        _ => "gray".to_string(),
    };
    let bpp = match mt.subtype {
        s if s == MEDIASUBTYPE_RGB24 => 24,
        s if s == MEDIASUBTYPE_RGB32 || s == MEDIASUBTYPE_ARGB32 => 32,
        s if s == MEDIASUBTYPE_MMDXSHOW_RGB32 => 32,
        s if s == MEDIASUBTYPE_RGB565 || s == MEDIASUBTYPE_RGB555 => 16,
        _ => 8,
    };
    Some(FormatInfo {
        width,
        height,
        bpp,
        bottom_up,
        pix_fmt,
        frame_dur,
    })
}

pub fn fourcc_guid(fourcc: u32) -> windows::core::GUID {
    windows::core::GUID {
        data1: fourcc,
        data2: 0x0000,
        data3: 0x0010,
        data4: [0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71],
    }
}

pub fn make_output_type(fmt: &FormatInfo, fourcc: u32) -> AM_MEDIA_TYPE {
    let cfg = crate::config::load();
    let fourcc = if cfg.out_fourcc.len() == 4 {
        let b = cfg.out_fourcc.as_bytes();
        mmio_fourcc(b[0], b[1], b[2], b[3])
    } else {
        fourcc
    };
    let mut vih = VIDEOINFOHEADER {
        rcSource: if cfg.out_rcsource_zero {
            windows::Win32::Foundation::RECT { left: 0, top: 0, right: 0, bottom: 0 }
        } else {
            windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: fmt.width,
                bottom: fmt.height,
            }
        },
        rcTarget: if cfg.out_rcsource_zero {
            windows::Win32::Foundation::RECT { left: 0, top: 0, right: 0, bottom: 0 }
        } else {
            windows::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: fmt.width,
                bottom: fmt.height,
            }
        },
        dwBitRate: cfg.out_bitrate.max(0) as u32,
        dwBitErrorRate: 0,
        AvgTimePerFrame: fmt.frame_dur,
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: fmt.width,
            // AVI Mux rejects compressed video types with a negative
            // (top-down) biHeight; use bottom-up (positive) like VCM
            // encoders do. The compressed stream is unaffected.
            biHeight: fmt.height,
            biPlanes: 1,
            biBitCount: 24,
            biCompression: fourcc as u32,
            biSizeImage: cfg.out_bisizeimage.max(0) as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
    };

    let mut fmt_buf = Vec::with_capacity(std::mem::size_of::<VIDEOINFOHEADER>() + cfg.out_cbextra.max(0) as usize);
    unsafe {
        let src = std::slice::from_raw_parts(
            (&vih as *const VIDEOINFOHEADER) as *const u8,
            std::mem::size_of::<VIDEOINFOHEADER>(),
        );
        fmt_buf.extend_from_slice(src);
    }
    fmt_buf.resize(fmt_buf.capacity(), 0);
    let pb = unsafe { CoTaskMemAlloc(fmt_buf.len()) } as *mut u8;
    if !pb.is_null() {
        unsafe {
            std::ptr::copy_nonoverlapping(fmt_buf.as_ptr(), pb, fmt_buf.len());
        }
    }

    AM_MEDIA_TYPE {
        majortype: MEDIATYPE_Video,
        subtype: fourcc_guid(fourcc),
        bFixedSizeSamples: false.into(),
        bTemporalCompression: true.into(),
        lSampleSize: cfg.out_lsample.max(0) as u32,
        formattype: FORMAT_VideoInfo,
        pUnk: core::mem::ManuallyDrop::new(None),
        cbFormat: fmt_buf.len() as u32,
        pbFormat: pb,
    }
}

pub fn clone_mt(mt: &AM_MEDIA_TYPE) -> AM_MEDIA_TYPE {
    let mut out = AM_MEDIA_TYPE {
        majortype: mt.majortype,
        subtype: mt.subtype,
        bFixedSizeSamples: mt.bFixedSizeSamples,
        bTemporalCompression: mt.bTemporalCompression,
        lSampleSize: mt.lSampleSize,
        formattype: mt.formattype,
        pUnk: core::mem::ManuallyDrop::new(None),
        cbFormat: 0,
        pbFormat: std::ptr::null_mut(),
    };
    if !mt.pbFormat.is_null() && mt.cbFormat > 0 {
        let pb = unsafe { CoTaskMemAlloc(mt.cbFormat as usize) } as *mut u8;
        if !pb.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(mt.pbFormat, pb, mt.cbFormat as usize);
            }
            out.pbFormat = pb;
            out.cbFormat = mt.cbFormat;
        }
    }
    out
}

/// Allocate an AM_MEDIA_TYPE (struct + format) on the COM task heap.
/// The caller frees it with CoTaskMemFree of the struct pointer after
/// freeing pbFormat (i.e. DirectShow's DeleteMediaType semantics).
pub unsafe fn alloc_mt_ptr(mt: &AM_MEDIA_TYPE) -> *mut AM_MEDIA_TYPE {
    let p = CoTaskMemAlloc(std::mem::size_of::<AM_MEDIA_TYPE>()) as *mut AM_MEDIA_TYPE;
    if p.is_null() {
        return std::ptr::null_mut();
    }
    let copy = clone_mt(mt);
    p.write(copy);
    p
}

pub unsafe fn free_mt(mt: &mut AM_MEDIA_TYPE) {
    if !mt.pbFormat.is_null() {
        CoTaskMemFree(Some(mt.pbFormat as *const _));
        mt.pbFormat = std::ptr::null_mut();
        mt.cbFormat = 0;
    }
}

pub fn mmio_fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}
