use core_foundation::{
    array::{CFArrayGetCount, CFArrayGetValueAtIndex},
    base::{FromVoid, TCFType},
    dictionary::{CFDictionaryGetValue, CFDictionaryRef},
    number::{kCFNumberIntType, CFBooleanGetValue, CFBooleanRef, CFNumberGetValue, CFNumberRef},
    string::CFString,
};
use core_graphics::{
    display::{
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionIncludingWindow,
        kCGWindowListOptionOnScreenOnly, CGDisplay, CGPoint, CGWindowListCopyWindowInfo,
    },
    geometry::CGRect,
    window::{kCGNullWindowID, kCGWindowSharingNone},
};
use image::RgbaImage;
use std::ffi::c_void;

use crate::{error::XCapResult, XCapError};

use super::{capture::capture, impl_monitor::ImplMonitor};

#[derive(Debug, Clone)]
pub(crate) struct ImplWindow {
    pub window_cf_dictionary_ref: CFDictionaryRef,
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub current_monitor: ImplMonitor,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_minimized: bool,
    pub is_maximized: bool,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGRectMakeWithDictionaryRepresentation(
        dict: CFDictionaryRef,
        rect: &mut CGRect,
    ) -> CFBooleanRef;
}

fn get_cf_dictionary_get_value(
    cf_dictionary_ref: CFDictionaryRef,
    key: &str,
) -> XCapResult<*const c_void> {
    unsafe {
        let cf_dictionary_key = CFString::new(key);

        let value = CFDictionaryGetValue(cf_dictionary_ref, cf_dictionary_key.as_CFTypeRef());

        if value.is_null() {
            return Err(XCapError::new(format!(
                "Get CFDictionary {} value failed",
                key
            )));
        }

        Ok(value)
    }
}

fn get_cf_number_u32_value(cf_dictionary_ref: CFDictionaryRef, key: &str) -> XCapResult<u32> {
    unsafe {
        let cf_number_ref = get_cf_dictionary_get_value(cf_dictionary_ref, key)?;

        let mut value: u32 = 0;
        let is_success = CFNumberGetValue(
            cf_number_ref as CFNumberRef,
            kCFNumberIntType,
            &mut value as *mut _ as *mut c_void,
        );

        if !is_success {
            return Err(XCapError::new(format!(
                "Get {} CFNumberGetValue failed",
                key
            )));
        }

        Ok(value)
    }
}

fn get_cf_string_value(cf_dictionary_ref: CFDictionaryRef, key: &str) -> XCapResult<String> {
    let value_ref = get_cf_dictionary_get_value(cf_dictionary_ref, key)?;

    Ok(unsafe { CFString::from_void(value_ref).to_string() })
}

fn get_cf_bool_value(cf_dictionary_ref: CFDictionaryRef, key: &str) -> XCapResult<bool> {
    let value_ref = get_cf_dictionary_get_value(cf_dictionary_ref, key)?;

    Ok(unsafe { CFBooleanGetValue(value_ref as CFBooleanRef) })
}

fn get_window_cg_rect(window_cf_dictionary_ref: CFDictionaryRef) -> XCapResult<CGRect> {
    unsafe {
        let window_bounds_ref =
            get_cf_dictionary_get_value(window_cf_dictionary_ref, "kCGWindowBounds")?
                as CFDictionaryRef;

        let mut cg_rect = CGRect::default();

        let is_success_ref =
            CGRectMakeWithDictionaryRepresentation(window_bounds_ref, &mut cg_rect);

        if is_success_ref.is_null() {
            return Err(XCapError::new(
                "CGRectMakeWithDictionaryRepresentation failed",
            ));
        }

        Ok(cg_rect)
    }
}

impl ImplWindow {
    pub fn new(
        window_cf_dictionary_ref: CFDictionaryRef,
        impl_monitors: &[ImplMonitor],
        window_name: String,
        window_owner_name: String,
    ) -> XCapResult<ImplWindow> {
        let id = get_cf_number_u32_value(window_cf_dictionary_ref, "kCGWindowNumber")?;
        let cg_rect = get_window_cg_rect(window_cf_dictionary_ref)?;

        let primary_monitor = ImplMonitor::new(CGDisplay::main().id)?;

        let (is_maximized, current_monitor) = {
            // 获取窗口中心点的坐标
            let window_center_x = cg_rect.origin.x + cg_rect.size.width / 2.0;
            let window_center_y = cg_rect.origin.y + cg_rect.size.height / 2.0;
            let cg_point = CGPoint {
                x: window_center_x,
                y: window_center_y,
            };

            let impl_monitor = impl_monitors
                .iter()
                .find(|impl_monitor| {
                    let display_bounds = impl_monitor.cg_display.bounds();
                    display_bounds.contains(&cg_point) || display_bounds.is_intersects(&cg_rect)
                })
                .unwrap_or(&primary_monitor);

            (
                cg_rect.size.width as u32 >= impl_monitor.width
                    && cg_rect.size.height as u32 >= impl_monitor.height,
                impl_monitor,
            )
        };

        let is_minimized =
            !get_cf_bool_value(window_cf_dictionary_ref, "kCGWindowIsOnscreen")? && !is_maximized;

        Ok(ImplWindow {
            window_cf_dictionary_ref,
            id,
            title: window_name,
            app_name: window_owner_name,
            current_monitor: current_monitor.clone(),
            x: cg_rect.origin.x as i32,
            y: cg_rect.origin.y as i32,
            width: cg_rect.size.width as u32,
            height: cg_rect.size.height as u32,
            is_minimized,
            is_maximized,
        })
    }

    pub fn all() -> XCapResult<Vec<ImplWindow>> {
        unsafe {
            let impl_monitors = ImplMonitor::all()?;
            let mut impl_windows = Vec::new();

            let cg_window_list_copy_window_info = CGWindowListCopyWindowInfo(
                kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
                kCGNullWindowID,
            );

            if cg_window_list_copy_window_info.is_null() {
                return Ok(impl_windows);
            }

            let num_windows = CFArrayGetCount(cg_window_list_copy_window_info);

            for i in 0..num_windows {
                let window_cf_dictionary_ref =
                    CFArrayGetValueAtIndex(cg_window_list_copy_window_info, i) as CFDictionaryRef;

                if window_cf_dictionary_ref.is_null() {
                    continue;
                }

                let window_name =
                    match get_cf_string_value(window_cf_dictionary_ref, "kCGWindowName") {
                        Ok(window_name) => window_name,
                        _ => continue,
                    };

                let window_owner_name =
                    match get_cf_string_value(window_cf_dictionary_ref, "kCGWindowOwnerName") {
                        Ok(window_owner_name) => window_owner_name,
                        _ => continue,
                    };

                if window_name.eq("StatusIndicator") && window_owner_name.eq("Window Server") {
                    continue;
                }

                let window_sharing_state = match get_cf_number_u32_value(
                    window_cf_dictionary_ref,
                    "kCGWindowSharingState",
                ) {
                    Ok(window_sharing_state) => window_sharing_state,
                    _ => continue,
                };

                if window_sharing_state == kCGWindowSharingNone {
                    continue;
                }

                if let Ok(impl_window) = ImplWindow::new(
                    window_cf_dictionary_ref,
                    &impl_monitors,
                    window_name.clone(),
                    window_owner_name.clone(),
                ) {
                    impl_windows.push(impl_window);
                } else {
                    log::error!(
                        "ImplWindow::new({:?}, {:?}, {:?}, {:?}) failed",
                        window_cf_dictionary_ref,
                        &impl_monitors,
                        &window_name,
                        &window_owner_name
                    );
                }
            }

            Ok(impl_windows)
        }
    }
}

impl ImplWindow {
    pub fn capture_image(&self) -> XCapResult<RgbaImage> {
        capture(
            get_window_cg_rect(self.window_cf_dictionary_ref)?,
            kCGWindowListOptionIncludingWindow,
            self.id,
        )
    }
}
