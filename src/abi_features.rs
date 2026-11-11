//! Provider-owned K91 manifests. Feature lists stay empty while the exact
//! alias-aware estimator and the role's scheduling/metadata pledge are pending.
#![allow(clippy::missing_safety_doc)]
use crate::launch::{error, status};
/// Published launch feature identifier; withheld until the whole estimator works.
pub const ESTIMATE_ACCURACY_1: &str = "estimate_accuracy/1";
const LAUNCH: &[u8] = br#"{"abi":"launch/0.9","features":[],"schema":"bennett.abi-features/1"}"#;
const SCHED: &[u8] = br#"{"abi":"sched/0.9","features":[],"schema":"bennett.abi-features/1"}"#;
unsafe fn query(manifest: &'static [u8], out: *mut *const u8, len: *mut usize) -> i32 {
    status(|| {
        if out.is_null() || len.is_null() {
            return Err(error(11, "§10.1: null feature-query output"));
        }
        // Validate both outputs before writing either. Static bytes require no
        // allocation, handles or mutable provider state and live until unload.
        unsafe {
            *out = manifest.as_ptr();
            *len = manifest.len();
        }
        Ok(())
    })
}
#[cfg_attr(feature = "abi-0-9", unsafe(no_mangle))]
pub unsafe extern "C" fn bennett_launch_features(out: *mut *const u8, len: *mut usize) -> i32 {
    unsafe { query(LAUNCH, out, len) }
}
#[cfg_attr(feature = "abi-0-9", unsafe(no_mangle))]
pub unsafe extern "C" fn bennett_sched_features(out: *mut *const u8, len: *mut usize) -> i32 {
    unsafe { query(SCHED, out, len) }
}
