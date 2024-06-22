#![no_std]
#![cfg_attr(feature = "nightly", feature(doc_cfg))]

#[cfg(feature = "std")]
extern crate std;

pub mod abi;
pub mod branding;
pub mod rt;
pub mod types;
mod util;

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub enum EFError {
    InternalError,
    AllocNoMem,
    AllocInvalidLayout,
}

pub type EFResult<T> = Result<types::EFCopy<T>, EFError>;
