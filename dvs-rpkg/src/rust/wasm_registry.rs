// AUTO-GENERATED — DO NOT EDIT.
//
// Produced on host by `miniextendr_write_wasm_registry`. Compiled on
// wasm32-* targets in place of the linkme distributed_slices.
//
// generator-version: 1
// content-hash:      5eb010af708522cf

use ::miniextendr_api::abi::mx_tag;
use ::miniextendr_api::ffi::{R_CallMethodDef, SEXP};
use ::miniextendr_api::registry::{AltrepRegistration, TraitDispatchEntry};
use ::core::ffi::c_void;

unsafe extern "C-unwind" {
    pub fn C_dvs_test_2_5_gib_bytes(_: SEXP) -> SEXP;
    pub fn C_dvs_add(_: SEXP, _: SEXP, _: SEXP, _: SEXP, _: SEXP, _: SEXP) -> SEXP;
    pub fn C_dvs_get(_: SEXP, _: SEXP, _: SEXP, _: SEXP, _: SEXP) -> SEXP;
    pub fn C_dvs_init(_: SEXP, _: SEXP, _: SEXP, _: SEXP, _: SEXP, _: SEXP) -> SEXP;
    pub fn C_dvs_status(_: SEXP, _: SEXP, _: SEXP, _: SEXP) -> SEXP;
    pub fn C_dvs_version(_: SEXP) -> SEXP;
    pub fn C_dvs_set_threads(_: SEXP, _: SEXP) -> SEXP;
    pub fn C_format_byte_size(_: SEXP, _: SEXP) -> SEXP;
    pub fn C_set_dvs_log_level(_: SEXP, _: SEXP) -> SEXP;
    pub fn C_ProgressBarCallback__new(_: SEXP) -> SEXP;
    pub fn C_dvs_status__match_arg_choices__status(_: SEXP) -> SEXP;
    pub fn C_dvs_init__match_arg_choices__compression(_: SEXP) -> SEXP;
    pub fn C_set_dvs_log_level__match_arg_choices__level(_: SEXP) -> SEXP;
}

unsafe extern "C" {
}

pub static MX_CALL_DEFS_WASM: &[R_CallMethodDef] = &[
    R_CallMethodDef {
        name: c"C_dvs_test_2_5_gib_bytes".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_dvs_test_2_5_gib_bytes) }),
        numArgs: 1,
    },
    R_CallMethodDef {
        name: c"C_dvs_add".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP, SEXP, SEXP, SEXP, SEXP) -> SEXP, _>(C_dvs_add) }),
        numArgs: 6,
    },
    R_CallMethodDef {
        name: c"C_dvs_get".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP, SEXP, SEXP, SEXP) -> SEXP, _>(C_dvs_get) }),
        numArgs: 5,
    },
    R_CallMethodDef {
        name: c"C_dvs_init".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP, SEXP, SEXP, SEXP, SEXP) -> SEXP, _>(C_dvs_init) }),
        numArgs: 6,
    },
    R_CallMethodDef {
        name: c"C_dvs_status".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP, SEXP, SEXP) -> SEXP, _>(C_dvs_status) }),
        numArgs: 4,
    },
    R_CallMethodDef {
        name: c"C_dvs_version".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_dvs_version) }),
        numArgs: 1,
    },
    R_CallMethodDef {
        name: c"C_dvs_set_threads".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP) -> SEXP, _>(C_dvs_set_threads) }),
        numArgs: 2,
    },
    R_CallMethodDef {
        name: c"C_format_byte_size".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP) -> SEXP, _>(C_format_byte_size) }),
        numArgs: 2,
    },
    R_CallMethodDef {
        name: c"C_set_dvs_log_level".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP, SEXP) -> SEXP, _>(C_set_dvs_log_level) }),
        numArgs: 2,
    },
    R_CallMethodDef {
        name: c"C_ProgressBarCallback__new".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_ProgressBarCallback__new) }),
        numArgs: 1,
    },
    R_CallMethodDef {
        name: c"C_dvs_status__match_arg_choices__status".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_dvs_status__match_arg_choices__status) }),
        numArgs: 1,
    },
    R_CallMethodDef {
        name: c"C_dvs_init__match_arg_choices__compression".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_dvs_init__match_arg_choices__compression) }),
        numArgs: 1,
    },
    R_CallMethodDef {
        name: c"C_set_dvs_log_level__match_arg_choices__level".as_ptr(),
        fun: Some(unsafe { ::core::mem::transmute::<unsafe extern "C-unwind" fn(SEXP) -> SEXP, _>(C_set_dvs_log_level__match_arg_choices__level) }),
        numArgs: 1,
    },
];

pub static MX_ALTREP_REGISTRATIONS_WASM: &[AltrepRegistration] = &[
];

pub static MX_TRAIT_DISPATCH_WASM: &[TraitDispatchEntry] = &[
];
