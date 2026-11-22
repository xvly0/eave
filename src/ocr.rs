use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{copy_nonoverlapping, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use windows::core::w;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DeleteDC,
    DeleteObject, EndPaint, FillRect, FrameRect, GetDC, InvalidateRect, ReleaseDC, SelectObject,
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow, TranslateMessage, GWLP_USERDATA,
    IDC_CROSS, LWA_ALPHA, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOW, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_RBUTTONDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP,
};

const CF_UNICODETEXT: u32 = 13;
static OCR_ACTIVE: AtomicBool = AtomicBool::new(false);

struct OverlayState {
    origin_x: i32,
    origin_y: i32,
    dragging: bool,
    start: POINT,
    current: POINT,
    result: Option<RECT>,
}

pub fn start_screen_ocr_async() {
    if OCR_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::Builder::new()
        .name("eave-ocr".to_string())
        .spawn(|| {
            if let Some(rect) = select_screen_region() {
                thread::sleep(Duration::from_millis(35));
                if let Some(text) = capture_and_ocr(rect) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        copy_text_to_clipboard(trimmed);
                        crate::osd_monitors::play_eave_chime();
                    }
                }
            }
            OCR_ACTIVE.store(false, Ordering::SeqCst);
        })
        .ok();
}

fn select_screen_region() -> Option<RECT> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let cls = w!("EaveOcrOverlay");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wnd_proc),
            hInstance: hinst.into(),
            hCursor: LoadCursorW(None, IDC_CROSS).unwrap_or_default(),
            lpszClassName: cls,
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);

        let mut state = Box::new(OverlayState {
            origin_x: vx,
            origin_y: vy,
            dragging: false,
            start: POINT::default(),
            current: POINT::default(),
            result: None,
        });
        let state_ptr = &mut *state as *mut OverlayState;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            cls,
            w!("Eave Smart OCR"),
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            None,
            None,
            hinst,
            None,
        )
        .ok()?;

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 108, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOW);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DestroyWindow(hwnd);
        state.result
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
    match msg {
        WM_KEYDOWN => {
            if wparam.0 as u16 == 0x1B {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if !ptr.is_null() {
                let st = &mut *ptr;
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                st.dragging = true;
                st.start = POINT { x, y };
                st.current = st.start;
                SetCapture(hwnd);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if !ptr.is_null() {
                let st = &mut *ptr;
                if st.dragging {
                    st.current = POINT {
                        x: (lparam.0 & 0xFFFF) as i16 as i32,
                        y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
                    };
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if !ptr.is_null() {
                let st = &mut *ptr;
                if st.dragging {
                    st.dragging = false;
                    let _ = ReleaseCapture();
                    let left = st.start.x.min(st.current.x) + st.origin_x;
                    let top = st.start.y.min(st.current.y) + st.origin_y;
                    let right = st.start.x.max(st.current.x) + st.origin_x;
                    let bottom = st.start.y.max(st.current.y) + st.origin_y;
                    if (right - left) >= 8 && (bottom - top) >= 8 {
                        st.result = Some(RECT {
                            left,
                            top,
                            right,
                            bottom,
                        });
                    }
                    PostQuitMessage(0);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let bg = CreateSolidBrush(COLORREF(0x00080808));
            FillRect(hdc, &ps.rcPaint, bg);
            let _ = DeleteObject(bg);

            if !ptr.is_null() {
                let st = &*ptr;
                if st.dragging {
                    let sel = RECT {
                        left: st.start.x.min(st.current.x),
                        top: st.start.y.min(st.current.y),
                        right: st.start.x.max(st.current.x),
                        bottom: st.start.y.max(st.current.y),
                    };
                    let inner = CreateSolidBrush(COLORREF(0x00262626));
                    FillRect(hdc, &sel, inner);
                    let _ = DeleteObject(inner);

                    let border = CreateSolidBrush(COLORREF(0x00FFFFFF));
                    FrameRect(hdc, &sel, border);
                    let _ = DeleteObject(border);
                }
            }
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn capture_and_ocr(rect: RECT) -> Option<String> {
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);
    let pixels = capture_bgra_region(rect.left, rect.top, w, h)?;

    // WinRT OCR requires RoInitialize on worker threads.
    let ro_ok = unsafe { RoInitialize(RO_INIT_MULTITHREADED).is_ok() };
    let out = run_winrt_ocr(&pixels, w, h);
    if ro_ok {
        unsafe { RoUninitialize() };
    }
    out
}

fn capture_bgra_region(x: i32, y: i32, w: i32, h: i32) -> Option<Vec<u8>> {
    unsafe {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_invalid() {
            ReleaseDC(None, screen_dc);
            return None;
        }

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = null_mut();
        let hbm = CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(mem_dc, hbm);
        let ok = BitBlt(mem_dc, 0, 0, w, h, screen_dc, x, y, SRCCOPY).is_ok();

        let len = (w as usize) * (h as usize) * 4;
        let mut out = if ok && !bits.is_null() {
            let mut buf = vec![0u8; len];
            copy_nonoverlapping(bits as *const u8, buf.as_mut_ptr(), len);
            Some(buf)
        } else {
            None
        };

        // Ensure opaque alpha for WinRT SoftwareBitmap.
        if let Some(ref mut buf) = out {
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }

        SelectObject(mem_dc, old);
        let _ = DeleteObject(hbm);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);
        out
    }
}

fn run_winrt_ocr(bgra: &[u8], w: i32, h: i32) -> Option<String> {
    let writer = DataWriter::new().ok()?;
    writer.WriteBytes(bgra).ok()?;
    let buffer = writer.DetachBuffer().ok()?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w, h).ok()?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().ok()?;
    let res = engine.RecognizeAsync(&bitmap).ok()?.get().ok()?;
    let lines = res.Lines().ok()?;
    let mut parts = Vec::new();
    for line in lines {
        if let Ok(t) = line.Text() {
            let s = t.to_string();
            if !s.trim().is_empty() {
                parts.push(s);
            }
        }
    }
    if parts.is_empty() {
        res.Text().ok().map(|t| t.to_string())
    } else {
        Some(parts.join("\n"))
    }
}

fn copy_text_to_clipboard(text: &str) {
    let wide: Vec<u16> = OsStr::new(text).encode_wide().chain(Some(0)).collect();
    let bytes = wide.len() * std::mem::size_of::<u16>();
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(hmem) = GlobalAlloc(GMEM_MOVEABLE, bytes) {
            let dst = GlobalLock(hmem) as *mut u16;
            if !dst.is_null() {
                copy_nonoverlapping(wide.as_ptr(), dst, wide.len());
                let _ = GlobalUnlock(hmem);
                let _ = SetClipboardData(CF_UNICODETEXT, windows::Win32::Foundation::HANDLE(hmem.0));
            } else {
                let _ = hmem;
            }
        }
        let _ = CloseClipboard();
    }
}
