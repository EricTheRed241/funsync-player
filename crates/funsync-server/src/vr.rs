//! The VR decision (`backend/routes/vr_api.py`, `detect_vr_format_for_video`):
//! the VR Format panel's projection first, then Mark as VR / Flat.

use crate::data::Library;
use crate::scan::Video;

pub fn is_vr(lib: &Library, v: &Video) -> bool {
    let l = lib.lib();
    if let Some(p) = l["vrFormat"][&v.path]["projection"].as_str() {
        return p != "flat";
    }
    l["manualVRType"][&v.path] == "vr"
}
