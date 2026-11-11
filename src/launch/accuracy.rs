//! Additive explicit-accuracy boundary; legacy entries retain their policy.
use super::*;
use crate::estimate::{AccuracyRequest, ExplicitAccuracy};

#[cfg_attr(feature = "abi-0-9", unsafe(no_mangle))]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn bennett_estimate_run_at_accuracy(
    program: *mut bennett_program_t,
    region: *const c_char,
    token: u64,
    variant: *const c_char,
    inherited_controls: u32,
    params: *const u8,
    params_len: usize,
    aliases: *const u8,
    aliases_len: usize,
    epsilon_decimal: *const c_char,
    out: *mut *mut bennett_estimate_t,
) -> i32 {
    status(|| {
        unsafe {
            *get_mut(out)? = ptr::null_mut();
        }
        let accuracy = ExplicitAccuracy::parse(unsafe { text(epsilon_decimal)? })
            .map_err(|e| error(ARG, e))?;
        let params = crate::invocation::parse_args(unsafe { text_buffer(params, params_len)? })
            .map_err(|e| error(ARG, e))?;
        let aliases =
            crate::token_aliases::parse(Some(unsafe { text_buffer(aliases, aliases_len)? }))
                .map_err(|e| error(ARG, e))?
                .ok_or_else(|| error(ARG, "§10.1: missing token_aliases"))?;
        let p = unsafe { get(program)? };
        let request = resolve_estimate_request(
            p,
            unsafe { text(region)? },
            token,
            unsafe { text(variant)? },
            inherited_controls,
            params,
            Some(aliases),
        )?;
        let request = AccuracyRequest::new(request, accuracy);
        if let Some(target) = p.resources.target() {
            let ids = crate::identity::compute(&p.obj).map_err(|e| error(ARG, e))?;
            let attached = crate::sched::object_schedule(&p.obj).map_err(|e| error(ARG, e))?;
            request
                .effective_schedule(&p.obj, &ids, target, attached.as_ref())
                .map_err(|e| error(10, e))?;
        }
        let function = p
            .obj
            .function(request.request().region())
            .expect("resolved estimate region");
        // No feature promise or fabricated resource/certificate result at this boundary.
        estimate_unavailable(p, function)
    })
}
