#![no_std]
#![feature(maybe_uninit_as_bytes, pointer_is_aligned)]

#[cfg(feature = "std")]
extern crate std;

#[cfg(all(feature = "mockrt_heap_alloc", feature = "mockrt_vla_alloc"))]
compile_error!("Feature \"mockrt_heap_alloc\" and feature \"mockrt_vla_alloc\" cannot be enabled at the same time");

#[cfg(not(any(feature = "mockrt_heap_alloc", feature = "mockrt_vla_alloc")))]
compile_error!("Requires either of \"mockrt_heap_alloc\" or \"mockrt_vla_alloc\" feature");

pub mod abi;
pub mod branding;
pub mod rt;
pub mod types;
pub mod util;

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub enum EFError {
    InternalError,
    AllocNoMem,
    AllocInvalidLayout,
}

pub type EFResult<T> = Result<types::EFCopy<T>, EFError>;
