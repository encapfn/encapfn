use core::cell::Cell;
use core::cmp::{PartialEq, PartialOrd};
use core::fmt::Debug;
use core::marker::PhantomData;
use core::sync::atomic::{AtomicU64, Ordering};

pub unsafe trait EFID: Debug {
    type Imprint: Debug + Copy + Clone + Eq + PartialEq + PartialOrd;

    fn get_imprint(&self) -> Self::Imprint;

    #[inline(always)]
    fn compare(a: &Self::Imprint, b: &Self::Imprint) -> bool {
        a == b
    }
}

/// TODO: Write docs
///
///
/// ```compile_fail
/// use encapfn::branding::{EFID, EFLifetimeBranding};
///
/// EFLifetimeBranding::new::<()>(move |brand_a| {
///     EFLifetimeBranding::new::<()>(move |brand_b| {
///	    assert!(!EFLifetimeBranding::compare(&brand_a.get_imprint(), &brand_b.get_imprint()));
///     });
/// });
/// ```
#[derive(Debug)]
pub struct EFLifetimeBranding<'id>(PhantomData<Cell<&'id ()>>);

impl EFLifetimeBranding<'_> {
    #[inline(always)]
    pub fn new<R>(f: impl for<'new_id> FnOnce(EFLifetimeBranding<'new_id>) -> R) -> R {
        f(EFLifetimeBranding(PhantomData))
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialOrd)]
pub struct EFLifetimeBrandingImprint<'id>(PhantomData<Cell<&'id ()>>);

impl<'id> PartialEq<EFLifetimeBrandingImprint<'id>> for EFLifetimeBrandingImprint<'id> {
    fn eq(&self, _rhs: &EFLifetimeBrandingImprint<'id>) -> bool {
        // Imprint is invariant over the `'id` lifetime. Thus, the fact that
        // we're provided two types with identical lifetimes means that the
        // imprint must have been issued from the same branded lifetime, no
        // runtime check required:
        true
    }
}

unsafe impl<'id> EFID for EFLifetimeBranding<'id> {
    type Imprint = EFLifetimeBrandingImprint<'id>;

    #[inline(always)]
    fn get_imprint(&self) -> Self::Imprint {
        EFLifetimeBrandingImprint(PhantomData)
    }
}

#[test]
fn test_lifetime_branding_equality() {
    EFLifetimeBranding::new::<()>(|brand| {
        let imprint_a = brand.get_imprint();
        let imprint_b = brand.get_imprint();
        assert!(EFLifetimeBranding::compare(&imprint_a, &imprint_b));
    })
}

static EF_RUNTIME_BRANDING_CTR: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct EFRuntimeBranding(u64);

impl EFRuntimeBranding {
    pub fn new() -> Self {
        let id = EF_RUNTIME_BRANDING_CTR
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |prev_id| {
                prev_id.checked_add(1)
            })
            .expect("Overflow generating new EFRuntimeBranding ID");

        EFRuntimeBranding(id)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd)]
pub struct EFRuntimeBrandingImprint(u64);

unsafe impl EFID for EFRuntimeBranding {
    type Imprint = EFRuntimeBrandingImprint;

    #[inline(always)]
    fn get_imprint(&self) -> Self::Imprint {
        EFRuntimeBrandingImprint(self.0)
    }
}
