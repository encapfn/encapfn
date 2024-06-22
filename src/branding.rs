use core::cell::Cell;
use core::marker::PhantomData;

pub unsafe trait EFID {}

pub struct EFLifetimeBranding<'id>(PhantomData<Cell<&'id ()>>);
unsafe impl<'id> EFID for EFLifetimeBranding<'id> {}

#[inline(always)]
pub fn new<R>(f: impl for<'new_id> FnOnce(EFLifetimeBranding<'new_id>) -> R) -> R {
    f(EFLifetimeBranding(PhantomData))
}
