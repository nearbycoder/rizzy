//! ImageProcessing::Vips uses an image object after Active Storage's loader
//! step. The libvips `thumbnail` command instead resizes on load, and its
//! pixels can differ from `thumbnail_image` even with the same options.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::path::Path;
use std::sync::OnceLock;

#[link(name = "vips")]
unsafe extern "C" {
    fn vips_init(argv0: *const c_char) -> c_int;
    fn vips_image_new_from_file(name: *const c_char, ...) -> *mut c_void;
    fn vips_autorot(input: *mut c_void, output: *mut *mut c_void, ...) -> c_int;
    fn vips_thumbnail_image(
        input: *mut c_void,
        output: *mut *mut c_void,
        width: c_int,
        ...
    ) -> c_int;
    fn vips_image_write_to_file(image: *mut c_void, name: *const c_char, ...) -> c_int;
    fn vips_error_buffer() -> *const c_char;
    fn vips_error_clear();
}

#[link(name = "gobject-2.0")]
unsafe extern "C" {
    fn g_object_unref(object: *mut c_void);
}

struct Image(*mut c_void);

impl Drop for Image {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { g_object_unref(self.0) };
        }
    }
}

fn last_error() -> String {
    unsafe {
        let pointer = vips_error_buffer();
        let message = if pointer.is_null() {
            "libvips failed".to_owned()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        };
        vips_error_clear();
        message
    }
}

pub(crate) fn render(
    source: &Path,
    target: &Path,
    width: u32,
    height: u32,
    fill: bool,
    all_frames: bool,
) -> Result<(), String> {
    static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();
    INITIALIZED
        .get_or_init(|| {
            if unsafe { vips_init(c"rizzy".as_ptr()) } == 0 {
                Ok(())
            } else {
                Err(last_error())
            }
        })
        .clone()?;
    let source = CString::new(source.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let target = CString::new(target.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    let width = c_int::try_from(width).map_err(|e| e.to_string())?;
    let height = c_int::try_from(height).map_err(|e| e.to_string())?;
    unsafe {
        let input = Image(if all_frames {
            vips_image_new_from_file(
                source.as_ptr(),
                c"n".as_ptr(),
                -1_i32,
                std::ptr::null::<c_void>(),
            )
        } else {
            vips_image_new_from_file(source.as_ptr(), std::ptr::null::<c_void>())
        });
        if input.0.is_null() {
            return Err(last_error());
        }
        let mut rotated = Image(std::ptr::null_mut());
        if vips_autorot(input.0, &mut rotated.0, std::ptr::null::<c_void>()) != 0 {
            return Err(last_error());
        }
        let mut resized = Image(std::ptr::null_mut());
        let status = if fill {
            vips_thumbnail_image(
                rotated.0,
                &mut resized.0,
                width,
                c"height".as_ptr(),
                height,
                c"crop".as_ptr(),
                1_i32, // VIPS_INTERESTING_CENTRE
                c"no-rotate".as_ptr(),
                1_i32,
                std::ptr::null::<c_void>(),
            )
        } else {
            vips_thumbnail_image(
                rotated.0,
                &mut resized.0,
                width,
                c"height".as_ptr(),
                height,
                c"size".as_ptr(),
                2_i32, // VIPS_SIZE_DOWN
                c"no-rotate".as_ptr(),
                1_i32,
                std::ptr::null::<c_void>(),
            )
        };
        if status != 0 {
            return Err(last_error());
        }
        if vips_image_write_to_file(resized.0, target.as_ptr(), std::ptr::null::<c_void>()) != 0 {
            return Err(last_error());
        }
    }
    Ok(())
}
