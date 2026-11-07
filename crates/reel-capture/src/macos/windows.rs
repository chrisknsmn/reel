//! Looking at windows through CoreGraphics, which needs no permission. Only their titles do.

use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CGRect};
use objc2_core_graphics::{
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
    kCGWindowBounds, kCGWindowOwnerPID,
};

use crate::cursor_log::Bounds;

fn window_info(id: u32) -> Option<CFRetained<CFDictionary>> {
    let list = CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, id)?;
    if CFArray::count(&list) == 0 {
        return None;
    }
    // SAFETY: the index is in range, and the array holds dictionaries, as documented.
    unsafe {
        let item = CFArray::value_at_index(&list, 0).cast::<CFDictionary>();
        let item = item.as_ref()?;
        Some(CFRetained::retain(item.into()))
    }
}

/// Whether a window still exists. A minimised window does.
#[must_use]
pub fn window_exists(id: u32) -> bool {
    window_info(id).is_some()
}

/// Where a window is, in global points.
#[must_use]
pub fn window_bounds(id: u32) -> Option<Bounds> {
    let info = window_info(id)?;
    // SAFETY: the key is a documented constant, and its value is documented to be a dictionary.
    unsafe {
        let key: &CFString = kCGWindowBounds;
        let value = CFDictionary::value(&info, (key as *const CFString).cast());
        let dictionary = value.cast::<CFDictionary>().as_ref()?;
        let mut rect = CGRect::default();
        CGRectMakeWithDictionaryRepresentation(Some(dictionary), &mut rect).then_some(Bounds {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }
}

/// Which process owns a window.
#[must_use]
pub fn window_owner(id: u32) -> Option<i32> {
    let info = window_info(id)?;
    // SAFETY: the key is a documented constant, and its value is documented to be a number.
    unsafe {
        let key: &CFString = kCGWindowOwnerPID;
        let value = CFDictionary::value(&info, (key as *const CFString).cast());
        let number = value.cast::<CFNumber>().as_ref()?;
        number.as_i32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_doesnt_exist_says_so() {
        assert!(!window_exists(u32::MAX));
        assert_eq!(window_bounds(u32::MAX), None);
        assert_eq!(window_owner(u32::MAX), None);
    }
}
