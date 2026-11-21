use crate::native_compositor::NativeFrame;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::GlobalFree,
        Graphics::Gdi::*,
        Storage::Xps::*,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::Controls::Dialogs::*,
    },
};

struct PrintDialog(PRINTDLGW);
impl Drop for PrintDialog {
    fn drop(&mut self) {
        unsafe {
            if !self.0.hDC.is_invalid() {
                let _ = DeleteDC(self.0.hDC);
            }
            if !self.0.hDevMode.is_invalid() {
                let _ = GlobalFree(Some(self.0.hDevMode));
            }
            if !self.0.hDevNames.is_invalid() {
                let _ = GlobalFree(Some(self.0.hDevNames));
            }
        }
    }
}

pub fn print(
    frames: &[NativeFrame],
    validate: impl FnOnce() -> Result<(), String> + Send,
) -> Result<Option<(i32, usize)>, String> {
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
                    .ok()
                    .map_err(|error| error.to_string())?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        unsafe { CoUninitialize() };
                    }
                }
                let _apartment = Apartment;
                let mut dialog = PrintDialog(PRINTDLGW {
                    lStructSize: std::mem::size_of::<PRINTDLGW>() as u32,
                    Flags: PD_RETURNDC
                        | PD_NOSELECTION
                        | PD_USEDEVMODECOPIESANDCOLLATE
                        | PD_HIDEPRINTTOFILE,
                    nMinPage: 1,
                    nMaxPage: frames.len() as u16,
                    nFromPage: 1,
                    nToPage: frames.len() as u16,
                    nCopies: 1,
                    ..Default::default()
                });
                if !unsafe { PrintDlgW(&mut dialog.0) }.as_bool() {
                    let error = unsafe { CommDlgExtendedError() }.0;
                    return if error == 0 {
                        Ok(None)
                    } else {
                        Err(format!("Windows printer dialog failed ({error:#x})."))
                    };
                }
                validate()?;
                let range = if dialog.0.Flags.contains(PD_PAGENUMS) {
                    page_range(
                        frames.len(),
                        dialog.0.nFromPage as usize,
                        dialog.0.nToPage as usize,
                    )?
                } else {
                    0..frames.len()
                };
                let dc = dialog.0.hDC;
                if dc.is_invalid() {
                    return Err("The selected printer did not return a device context.".into());
                }
                let info = DOCINFOW {
                    cbSize: std::mem::size_of::<DOCINFOW>() as i32,
                    lpszDocName: w!("Presenter Orbit handout"),
                    lpszOutput: PCWSTR::null(),
                    ..Default::default()
                };
                let job = unsafe { StartDocW(dc, &info) };
                if job <= 0 {
                    return Err(format!(
                        "Printer rejected the document: {}",
                        std::io::Error::last_os_error()
                    ));
                }
                let result = (|| {
                    for index in range.clone() {
                        if unsafe { StartPage(dc) } <= 0 {
                            return Err("Printer could not start a page.".into());
                        }
                        draw(dc, &frames[index])?;
                        if unsafe { EndPage(dc) } <= 0 {
                            return Err("Printer rejected a page.".into());
                        }
                    }
                    if unsafe { EndDoc(dc) } <= 0 {
                        return Err("Printer could not finish the document.".into());
                    }
                    Ok(Some((job, range.len())))
                })();
                if result.is_err() {
                    unsafe { AbortDoc(dc) };
                }
                result
            })
            .join()
            .map_err(|_| "Windows printer thread failed.".to_string())?
    })
}

fn page_range(total: usize, first: usize, last: usize) -> Result<std::ops::Range<usize>, String> {
    if first == 0 || last < first || last > total {
        return Err("Printer page range is outside the handout.".into());
    }
    Ok(first - 1..last)
}

fn fit(
    width: usize,
    height: usize,
    available_width: i32,
    available_height: i32,
) -> Result<(i32, i32, i32, i32), String> {
    if width == 0 || height == 0 || available_width <= 0 || available_height <= 0 {
        return Err("Printer or handout page dimensions are invalid.".into());
    }
    let scale =
        (available_width as f64 / width as f64).min(available_height as f64 / height as f64);
    let w = (width as f64 * scale).round().max(1.0) as i32;
    let h = (height as f64 * scale).round().max(1.0) as i32;
    Ok(((available_width - w) / 2, (available_height - h) / 2, w, h))
}

fn draw(dc: HDC, frame: &NativeFrame) -> Result<(), String> {
    if frame
        .width
        .checked_mul(frame.height)
        .and_then(|n| n.checked_mul(4))
        != Some(frame.pixels.len())
    {
        return Err("Handout page pixels are invalid.".into());
    }
    let (x, y, width, height) = fit(
        frame.width,
        frame.height,
        unsafe { GetDeviceCaps(Some(dc), HORZRES) },
        unsafe { GetDeviceCaps(Some(dc), VERTRES) },
    )?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: frame.width as i32,
            biHeight: -(frame.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe { SetStretchBltMode(dc, HALFTONE) };
    let lines = unsafe {
        StretchDIBits(
            dc,
            x,
            y,
            width,
            height,
            0,
            0,
            frame.width as i32,
            frame.height as i32,
            Some(frame.pixels.as_ptr().cast()),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    if lines <= 0 {
        return Err("Printer could not render the handout page.".into());
    }
    Ok(())
}
