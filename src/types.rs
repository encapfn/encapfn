use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::Deref;

use crate::branding::EFID;
use crate::util::maybe_uninit_as_bytes;

// Both of these should issue warnings when enabled:
const DISABLE_UPGRADE_CHECKS: bool = cfg!(feature = "disable_upgrade_checks");
const DISABLE_VALIDATION_CHECKS: bool = cfg!(feature = "disable_validation_checks");

pub unsafe trait AllocTracker {
    fn is_valid(&self, ptr: *const (), len: usize) -> bool;
    fn is_valid_mut(&self, ptr: *mut (), len: usize) -> bool;
}

pub struct AllocScope<'a, T: AllocTracker, ID: EFID> {
    tracker: T,
    id_imprint: ID::Imprint,
    _lt: PhantomData<&'a ()>,
}

impl<'a, T: AllocTracker, ID: EFID> AllocScope<'a, T, ID> {
    pub unsafe fn new(tracker: T, id_imprint: ID::Imprint) -> Self {
        AllocScope {
            tracker,
            id_imprint,
            _lt: PhantomData,
        }
    }

    pub fn tracker(&self) -> &T {
        &self.tracker
    }

    pub fn tracker_mut(&mut self) -> &mut T {
        &mut self.tracker
    }

    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }
}

pub struct AccessScope<ID: EFID> {
    id_imprint: ID::Imprint,
}

impl<ID: EFID> AccessScope<ID> {
    pub unsafe fn new(id_imprint: ID::Imprint) -> Self {
        AccessScope { id_imprint }
    }

    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }
}

pub unsafe trait EFType {
    unsafe fn validate(t: *const Self) -> bool;
}

// -----------------------------------------------------------------------------

#[derive(Debug)]
#[repr(transparent)]
pub struct EFPtr<T: 'static>(pub *mut T);

impl<T: 'static> Clone for EFPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for EFPtr<T> {}

impl<T: 'static> From<*mut T> for EFPtr<T> {
    fn from(ptr: *mut T) -> Self {
        EFPtr(ptr)
    }
}

impl<T: 'static> From<*const T> for EFPtr<T> {
    fn from(ptr: *const T) -> Self {
        EFPtr(ptr as *mut T)
    }
}

impl<T: 'static> From<usize> for EFPtr<T> {
    fn from(ptr: usize) -> Self {
        EFPtr(ptr as *mut T)
    }
}

impl<T: 'static> From<EFPtr<T>> for *mut T {
    fn from(efmutptr: EFPtr<T>) -> Self {
        efmutptr.0
    }
}

impl<T: 'static> From<EFPtr<T>> for *const T {
    fn from(efmutptr: EFPtr<T>) -> Self {
        efmutptr.0 as *const T
    }
}

impl<T: 'static> From<EFPtr<T>> for usize {
    fn from(efmutptr: EFPtr<T>) -> Self {
        efmutptr.0 as usize
    }
}

impl<T: 'static> EFPtr<T> {
    pub fn null() -> Self {
        EFPtr(core::ptr::null_mut())
    }

    pub fn cast<U: 'static>(self) -> EFPtr<U> {
        EFPtr(self.0 as *mut U)
    }

    pub unsafe fn upgrade_unchecked<'alloc, ID: EFID>(
        &self,
        id_imprint: ID::Imprint,
    ) -> EFRef<'alloc, ID, T> {
        EFRef {
            r: &*(self.0 as *mut UnsafeCell<MaybeUninit<T>> as *const _),
            id_imprint,
        }
    }

    pub fn upgrade<'alloc, R: AllocTracker, ID: EFID>(
        &self,
        alloc_scope: &AllocScope<'alloc, R, ID>,
    ) -> Option<EFRef<'alloc, ID, T>> {
        if DISABLE_UPGRADE_CHECKS {
            Some(unsafe { self.upgrade_unchecked(alloc_scope.id_imprint()) })
        } else {
            if self.0.is_aligned()
                && alloc_scope
                    .tracker()
                    .is_valid(self.0 as *const (), core::mem::size_of::<T>())
            {
                Some(unsafe { self.upgrade_unchecked(alloc_scope.id_imprint()) })
            } else {
                None
            }
        }
    }

    pub unsafe fn upgrade_unchecked_mut<'alloc, ID: EFID>(
        &self,
        id_imprint: ID::Imprint,
    ) -> EFMutRef<'alloc, ID, T> {
        EFMutRef {
            r: &*(self.0 as *mut UnsafeCell<MaybeUninit<T>> as *const _),
            id_imprint,
        }
    }

    pub fn upgrade_mut<'alloc, R: AllocTracker, ID: EFID>(
        &self,
        alloc_scope: &AllocScope<'alloc, R, ID>,
    ) -> Option<EFMutRef<'alloc, ID, T>> {
        if DISABLE_UPGRADE_CHECKS {
            Some(unsafe { self.upgrade_unchecked_mut(alloc_scope.id_imprint()) })
        } else {
            if self.0.is_aligned()
                && alloc_scope
                    .tracker()
                    .is_valid_mut(self.0 as *mut (), core::mem::size_of::<T>())
            {
                Some(unsafe { self.upgrade_unchecked_mut(alloc_scope.id_imprint()) })
            } else {
                None
            }
        }
    }

    pub unsafe fn upgrade_unchecked_slice<'alloc, ID: EFID>(
        &self,
        length: usize,
        id_imprint: ID::Imprint,
    ) -> EFSlice<'alloc, ID, T> {
        // TODO: check soudness. Is it always safe to have a [MaybeUninit<T>],
        // when it would be safe to have a MaybeUninit<[T]>, for which the
        // length is valid and initialized?
        EFSlice {
            r: core::slice::from_raw_parts(
                self.0 as *mut _ as *mut UnsafeCell<MaybeUninit<T>>,
                length,
            ),
            id_imprint,
        }
    }

    pub fn upgrade_slice<'alloc, R: AllocTracker, ID: EFID>(
        &self,
        length: usize,
        alloc_scope: &AllocScope<'alloc, R, ID>,
    ) -> Option<EFSlice<'alloc, ID, T>> {
        if DISABLE_UPGRADE_CHECKS {
            Some(unsafe { self.upgrade_unchecked_slice(length, alloc_scope.id_imprint()) })
        } else {
            // As per Rust reference, "An array of [T; N] has a size of
            // size_of::<T>() * N and the same alignment of T", and, "Slices
            // have the same layout as the section of the array they slice", so
            // checking for alignment of T is sufficient.
            //
            // Furthermore, for `std::mem::size_of`, the function documentation
            // reads:
            //
            //     More specifically, this is the offset in bytes between
            //     successive elements in an array with that item type including
            //     alignment padding. Thus, for any type T and length n, [T; n]
            //     has a size of n * size_of::<T>().
            //
            // Hence we perform the check for exactly this expression:
            if self.0.is_aligned()
                && alloc_scope
                    .tracker()
                    .is_valid(self.0 as *const (), length * core::mem::size_of::<T>())
            {
                Some(unsafe { self.upgrade_unchecked_slice(length, alloc_scope.id_imprint()) })
            } else {
                None
            }
        }
    }

    pub unsafe fn upgrade_unchecked_slice_mut<'alloc, ID: EFID>(
        &self,
        length: usize,
        id_imprint: ID::Imprint,
    ) -> EFMutSlice<'alloc, ID, T> {
        // TODO: check soudness. Is it always safe to have a [MaybeUninit<T>],
        // when it would be safe to have a MaybeUninit<[T]>, for which the
        // length is valid and initialized?
        EFMutSlice {
            r: core::slice::from_raw_parts(
                self.0 as *mut _ as *mut UnsafeCell<MaybeUninit<T>>,
                length,
            ),
            id_imprint,
        }
    }

    pub fn upgrade_slice_mut<'alloc, R: AllocTracker, ID: EFID>(
        &self,
        length: usize,
        alloc_scope: &AllocScope<'alloc, R, ID>,
    ) -> Option<EFMutSlice<'alloc, ID, T>> {
        if DISABLE_UPGRADE_CHECKS {
            Some(unsafe { self.upgrade_unchecked_slice_mut(length, alloc_scope.id_imprint()) })
        } else {
            // As per Rust reference, "An array of [T; N] has a size of
            // size_of::<T>() * N and the same alignment of T", and, "Slices
            // have the same layout as the section of the array they slice", so
            // checking for alignment of T is sufficient.
            //
            // Furthermore, for `std::mem::size_of`, the function documentation reads:
            //
            //     More specifically, this is the offset in bytes between
            //     successive elements in an array with that item type including
            //     alignment padding. Thus, for any type T and length n, [T; n]
            //     has a size of n * size_of::<T>().
            //
            // Hence we perform the check for exactly this expression:
            if self.0.is_aligned()
                && alloc_scope
                    .tracker()
                    .is_valid_mut(self.0 as *mut (), length * core::mem::size_of::<T>())
            {
                Some(unsafe { self.upgrade_unchecked_slice_mut(length, alloc_scope.id_imprint()) })
            } else {
                None
            }
        }
    }
}

// -----------------------------------------------------------------------------

// An owned copy from some unvalidated foreign memory
#[repr(transparent)]
pub struct EFCopy<T: 'static>(MaybeUninit<T>);

impl<T: 'static> core::fmt::Debug for EFCopy<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.pad(core::any::type_name::<Self>())
    }
}

// Need to manually implement Clone, Rust won't derive it automatically because
// not necessarily T: Clone.
impl<T: 'static> Clone for EFCopy<T> {
    fn clone(&self) -> Self {
        // Do a safe byte-wise copy of the MaybeUninit. It does not necessarily
        // implement Copy. However, we only support dereferencing it after being
        // validated through EFType, and that must never support validating a
        // value that is not safely copy-able.
        let mut clone = MaybeUninit::<T>::uninit();
        maybe_uninit_as_bytes::as_bytes_mut(&mut clone)
            .copy_from_slice(maybe_uninit_as_bytes::as_bytes(&self.0));
        EFCopy(clone)
    }
}

impl<T: 'static> From<MaybeUninit<T>> for EFCopy<T> {
    fn from(from: MaybeUninit<T>) -> Self {
        EFCopy(from)
    }
}

impl<T: 'static> EFCopy<T> {
    pub fn new(val: T) -> Self {
        EFCopy(MaybeUninit::new(val))
    }

    // TODO: does this need to be unsafe? Presumably yes, based on my
    // interpretation of a conversation with Ralf. Document safety invariants!
    pub unsafe fn uninit() -> Self {
        EFCopy(MaybeUninit::uninit())
    }

    pub fn zeroed() -> Self {
        EFCopy(MaybeUninit::zeroed())
    }

    pub fn update_from_ref<ID: EFID>(
        &mut self,
        r: EFRef<'_, ID, T>,
        access_scope: &AccessScope<ID>,
    ) {
        if r.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                r.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: taking &AccessScope<ID> ensures that no mutable accessible
        // references into foreign memory exist, and that no foreign code is
        // accessing this memory. The existance of this type ensures that this
        // memory is mutably accessible and well-aligned.
        maybe_uninit_as_bytes::as_bytes_mut(&mut self.0)
            .copy_from_slice(maybe_uninit_as_bytes::as_bytes(unsafe { &*r.r.get() }));
    }

    pub fn update_from_mut_ref<ID: EFID>(
        &mut self,
        r: EFMutRef<'_, ID, T>,
        access_scope: &AccessScope<ID>,
    ) {
        if r.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                r.id_imprint,
                access_scope.id_imprint()
            );
        }

        self.update_from_ref(r.as_immut(), access_scope)
    }

    pub unsafe fn assume_valid(self) -> T {
        self.0.assume_init()
    }

    pub unsafe fn assume_valid_ref(&self) -> &T {
        self.0.assume_init_ref()
    }
}

impl<T: EFType + 'static> EFCopy<T> {
    pub fn validate(self) -> Result<T, Self> {
        if DISABLE_VALIDATION_CHECKS {
            Ok(unsafe { self.assume_valid() })
        } else {
            if unsafe { <T as EFType>::validate(&self.0 as *const MaybeUninit<T> as *const T) } {
                Ok(unsafe { self.0.assume_init() })
            } else {
                Err(self)
            }
        }
    }

    pub fn validate_ref<'a>(&'a self) -> Option<&'a T> {
        if DISABLE_VALIDATION_CHECKS {
            Some(unsafe { self.assume_valid_ref() })
        } else {
            if unsafe { <T as EFType>::validate(&self.0 as *const MaybeUninit<T> as *const T) } {
                Some(unsafe { self.0.assume_init_ref() })
            } else {
                None
            }
        }
    }

    pub fn validate_copy(&self) -> Option<T> {
        // TODO: maybe more efficient to validate ref first, then clone:
        let cloned = self.clone();
        cloned.validate().ok()
    }
}

// -----------------------------------------------------------------------------

// A reference which is validated to be well-aligned and contained in
// mutably-accessible memory.
pub struct EFMutRef<'alloc, ID: EFID, T: 'static> {
    r: &'alloc UnsafeCell<MaybeUninit<T>>,
    id_imprint: ID::Imprint,
}

impl<'alloc, ID: EFID, T: 'static> EFMutRef<'alloc, ID, T> {
    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }
}

impl<'alloc, ID: EFID, T: 'static> Clone for EFMutRef<'alloc, ID, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'alloc, ID: EFID, T: 'static> Copy for EFMutRef<'alloc, ID, T> {}

impl<'alloc, ID: EFID, T: EFType + 'static> EFMutRef<'alloc, ID, T> {
    pub fn validate<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> Option<EFVal<'alloc, 'access, ID, T>> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if DISABLE_VALIDATION_CHECKS {
            Some(unsafe { self.assume_valid(access_scope) })
        } else {
            if unsafe {
                <T as EFType>::validate(self.r as *const UnsafeCell<MaybeUninit<T>> as *const T)
            } {
                Some(unsafe { self.assume_valid(access_scope) })
            } else {
                None
            }
        }
    }
}

impl<'alloc, ID: EFID, T: 'static> EFMutRef<'alloc, ID, T> {
    pub unsafe fn assume_valid<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> EFVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        EFVal {
            r: &*(self.r as *const _ as *const T),
            id_imprint: self.id_imprint,
            _alloc_lt: PhantomData,
        }
    }

    pub unsafe fn sub_ref_unchecked<U: 'static>(
        self,
        byte_offset: usize,
    ) -> EFMutRef<'alloc, ID, U> {
        EFMutRef {
            r: unsafe {
                &*((self.r as *const UnsafeCell<MaybeUninit<T>>).byte_add(byte_offset)
                    as *const UnsafeCell<MaybeUninit<U>>)
            },
            id_imprint: self.id_imprint,
        }
    }

    pub fn as_ptr(&self) -> EFPtr<T> {
        EFPtr(self.r as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T)
    }

    pub fn as_immut(&self) -> EFRef<'alloc, ID, T> {
        EFRef {
            r: self.r,
            id_imprint: self.id_imprint,
        }
    }

    pub fn write<'access>(
        &self,
        val: T,
        access_scope: &'access mut AccessScope<ID>,
    ) -> EFVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: taking &mut AccessScope<ID> ensures that no other accessible
        // references into foreign memory exist, and that no foreign code is
        // accessing this memory. The existance of this type ensures that this
        // memory is mutably accessible and well-aligned.
        (unsafe { &mut *self.r.get() }).write(val);

        // Provide a validated reference to the newly written memory, bound to
        // 'access. We know that the reference must have a valid value right
        // now, based on the knowledge that `val` was a valid instance of T:
        unsafe { self.assume_valid(access_scope) }
    }

    pub fn write_copy<'access>(
        &self,
        copy: &EFCopy<T>,
        access_scope: &'access mut AccessScope<ID>,
    ) {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: taking &mut AccessScope<ID> ensures that no other accessible
        // references into foreign memory exist, and that no foreign code is
        // accessing this memory. The existance of this type ensures that this
        // memory is mutably accessible and well-aligned.
        maybe_uninit_as_bytes::as_bytes_mut(unsafe { &mut *self.r.get() })
            .copy_from_slice(maybe_uninit_as_bytes::as_bytes(&copy.0))
    }

    pub fn copy<'access>(&self, access_scope: &'access AccessScope<ID>) -> EFCopy<T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: we're overwriting the uninit immediately with known values,
        // and hence never creating a non-MaybeUninit reference to uninitialized
        // memory:
        let mut copy = unsafe { EFCopy::<T>::uninit() };
        copy.update_from_mut_ref(*self, access_scope);
        copy
    }
}

impl<'alloc, ID: EFID, T: Copy + 'static> EFMutRef<'alloc, ID, T> {
    pub fn write_ref<'access>(
        &self,
        val: &T,
        access_scope: &'access mut AccessScope<ID>,
    ) -> EFVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: taking &mut AccessScope<ID> ensures that no other accessible
        // references into foreign memory exist, and that no foreign code is
        // accessing this memory. The existance of this type ensures that this
        // memory is mutably accessible and well-aligned.
        //
        // TODO: need to ensure that this does not create any imtermediate full
        // copies on the stack. It should copy directly from the reference
        // (effectively a memcpy):
        (unsafe { &mut *self.r.get() }).write(*val);

        // Provide a validated reference to the newly written memory, bound to
        // 'access. We know that the reference must have a valid value right
        // now, based on the knowledge that `val` was a valid instance of T:
        unsafe { self.assume_valid(access_scope) }
    }
}

impl<'alloc, const N: usize, ID: EFID, T: 'static> EFMutRef<'alloc, ID, [T; N]> {
    pub fn len(&self) -> usize {
        N
    }

    pub unsafe fn get_unchecked(&self, idx: usize) -> EFMutRef<'alloc, ID, T> {
        EFMutRef {
            r: &*((self.r as *const UnsafeCell<MaybeUninit<[T; N]>>
                as *const UnsafeCell<MaybeUninit<T>>)
                .add(idx)),
            id_imprint: self.id_imprint,
        }
    }

    pub fn get(&self, idx: usize) -> Option<EFMutRef<'alloc, ID, T>> {
        if idx < N {
            Some(unsafe { self.get_unchecked(idx) })
        } else {
            None
        }
    }

    pub fn iter(&self) -> EFMutRefIter<'alloc, ID, N, T> {
        EFMutRefIter {
            inner: self.clone(),
            idx: 0,
        }
    }
}

impl<'alloc, const N: usize, ID: EFID, T: 'static + Copy> EFMutRef<'alloc, ID, [T; N]> {
    pub fn copy_from_slice<'access>(&self, src: &[T], access_scope: &'access mut AccessScope<ID>) {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if src.len() != N {
            // Meaningful error message, and optimize panic with a cold
            // function:
            panic!(
                "Called EFMutRef::<[_; {}]>::copy_from_slice with a slice of length {}",
                N,
                src.len()
            );
        }

        self.iter().zip(src.iter()).for_each(|(dst, src)| {
            dst.write(*src, access_scope);
        })
    }
}

pub struct EFMutRefIter<'alloc, ID: EFID, const N: usize, T: 'static> {
    inner: EFMutRef<'alloc, ID, [T; N]>,
    idx: usize,
}

impl<'alloc, ID: EFID, const N: usize, T: 'static> core::iter::Iterator
    for EFMutRefIter<'alloc, ID, N, T>
{
    type Item = EFMutRef<'alloc, ID, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.inner.get(self.idx) {
            // Prevent wraparound by calling .iter() a bunch.
            self.idx += 1;
            Some(item)
        } else {
            None
        }
    }
}

pub struct EFMutSlice<'alloc, ID: EFID, T: 'static> {
    // The length of this slice is encoded in the reference itself (fat
    // pointer), and not located in / accessible to foreign memory:
    pub r: &'alloc [UnsafeCell<MaybeUninit<T>>],
    id_imprint: ID::Imprint,
}

impl<'alloc, ID: EFID, T: 'static> EFMutSlice<'alloc, ID, T> {
    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }

    pub unsafe fn assume_valid<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> EFSliceVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        EFSliceVal {
            r: core::mem::transmute::<&[UnsafeCell<MaybeUninit<T>>], &[MaybeUninit<T>]>(&self.r),
            _id_imprint: self.id_imprint,
            _alloc_lt: PhantomData,
        }
    }

    pub fn as_ptr(&self) -> EFPtr<T> {
        EFPtr(self.r as *const _ as *mut [UnsafeCell<MaybeUninit<T>>] as *mut T)
    }

    pub fn as_immut(&self) -> EFSlice<'alloc, ID, T> {
        EFSlice {
            r: self.r,
            id_imprint: self.id_imprint,
        }
    }

    pub fn len(&self) -> usize {
        self.r.len()
    }

    pub fn write_from_iter<'access, I: Iterator<Item = T>>(
        &self,
        src: I,
        access_scope: &'access AccessScope<ID>,
    ) -> EFSliceVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: taking &mut AccessScope<ID> ensures that no other accessible
        // references into foreign memory exist, and that no foreign code is
        // accessing this memory. The existance of this type ensures that this
        // memory is mutably accessible and well-aligned.
        let mut count = 0;
        self.r.iter().zip(src).for_each(|(dst, val)| {
            (unsafe { &mut *dst.get() }).write(val);
            count += 1;
        });
        assert!(count == self.r.len());

        // Provide a validated reference to the newly written memory, bound to
        // 'access. We know that the reference must have a valid value right
        // now, based on the knowledge that every element of `src` was a valid
        // instance of T, and we copied self.r.len() elements from `src`:
        unsafe { self.assume_valid(access_scope) }
    }
}

impl<'alloc, ID: EFID, T: EFType + 'static> EFMutSlice<'alloc, ID, T> {
    pub fn validate<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> Option<EFSliceVal<'alloc, 'access, ID, T>> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if DISABLE_VALIDATION_CHECKS {
            Some(unsafe { self.assume_valid(access_scope) })
        } else {
            if self
                .r
                .iter()
                .all(|elem: &UnsafeCell<MaybeUninit<T>>| unsafe {
                    <T as EFType>::validate(elem as *const UnsafeCell<MaybeUninit<T>> as *const T)
                })
            {
                Some(unsafe { self.assume_valid(access_scope) })
            } else {
                None
            }
        }
    }
}

impl<'alloc, ID: EFID, T: Copy + 'static> EFMutSlice<'alloc, ID, T> {
    pub fn copy_from_slice<'access>(
        &self,
        src: &[T],
        access_scope: &'access AccessScope<ID>,
    ) -> EFSliceVal<'alloc, 'access, ID, T> {
        self.write_from_iter(src.iter().copied(), access_scope)
    }
}

// -----------------------------------------------------------------------------

// A reference which is validated to be well-aligned and contained in
// (im)mutably-accessible memory. It may still be mutable by foreign code, and
// hence we assume interior mutability here:
pub struct EFRef<'alloc, ID: EFID, T: 'static> {
    r: &'alloc UnsafeCell<MaybeUninit<T>>,
    id_imprint: ID::Imprint,
}

impl<'alloc, ID: EFID, T: 'static> Clone for EFRef<'alloc, ID, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'alloc, ID: EFID, T: 'static> Copy for EFRef<'alloc, ID, T> {}

impl<'alloc, ID: EFID, T: EFType + 'static> EFRef<'alloc, ID, T> {
    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }

    pub unsafe fn sub_ref_unchecked<U: 'static>(self, byte_offset: usize) -> EFRef<'alloc, ID, U> {
        EFRef {
            r: unsafe {
                &*((self.r as *const UnsafeCell<MaybeUninit<T>>).byte_add(byte_offset)
                    as *const UnsafeCell<MaybeUninit<U>>)
            },
            id_imprint: self.id_imprint,
        }
    }

    pub fn validate<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> Option<EFVal<'alloc, 'access, ID, T>> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if DISABLE_VALIDATION_CHECKS {
            Some(unsafe { self.assume_valid(access_scope) })
        } else {
            if unsafe {
                <T as EFType>::validate(self.r as *const UnsafeCell<MaybeUninit<T>> as *const T)
            } {
                Some(unsafe { self.assume_valid(access_scope) })
            } else {
                None
            }
        }
    }
}

impl<'alloc, ID: EFID, T: 'static> EFRef<'alloc, ID, T> {
    pub unsafe fn assume_valid<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> EFVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        EFVal {
            r: &*(self.r as *const _ as *const T),
            id_imprint: self.id_imprint,
            _alloc_lt: PhantomData,
        }
    }

    pub fn as_ptr(&self) -> EFPtr<T> {
        EFPtr(self.r as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T)
    }

    pub fn copy<'access>(&self, access_scope: &'access AccessScope<ID>) -> EFCopy<T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        // Safety: we're overwriting the uninit immediately with known values,
        // and hence never creating a non-MaybeUninit reference to uninitialized
        // memory:
        let mut copy = unsafe { EFCopy::<T>::uninit() };
        copy.update_from_ref(*self, access_scope);
        copy
    }
}

impl<'alloc, const N: usize, ID: EFID, T: 'static> EFRef<'alloc, ID, [T; N]> {
    pub fn len(&self) -> usize {
        N
    }

    pub unsafe fn get_unchecked(&self, idx: usize) -> EFRef<'alloc, ID, T> {
        EFRef {
            r: &*((self.r as *const UnsafeCell<MaybeUninit<[T; N]>>
                as *const UnsafeCell<MaybeUninit<T>>)
                .add(idx)),
            id_imprint: self.id_imprint,
        }
    }

    pub fn get(&self, idx: usize) -> Option<EFRef<'alloc, ID, T>> {
        if idx < N {
            Some(unsafe { self.get_unchecked(idx) })
        } else {
            None
        }
    }

    pub fn iter(&self) -> EFRefIter<'alloc, ID, N, T> {
        EFRefIter {
            inner: self.clone(),
            idx: 0,
        }
    }

    pub fn as_slice(&self) -> EFSlice<'alloc, ID, T> {
        EFSlice {
            r: unsafe {
                core::slice::from_raw_parts(
                    self.r as *const _ as *const UnsafeCell<MaybeUninit<T>>,
                    N,
                )
            },
            id_imprint: self.id_imprint,
        }
    }
}

pub struct EFRefIter<'alloc, ID: EFID, const N: usize, T: 'static> {
    inner: EFRef<'alloc, ID, [T; N]>,
    idx: usize,
}

impl<'alloc, ID: EFID, const N: usize, T: 'static> core::iter::Iterator
    for EFRefIter<'alloc, ID, N, T>
{
    type Item = EFRef<'alloc, ID, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.inner.get(self.idx) {
            // Prevent wraparound by calling .iter() a bunch.
            self.idx += 1;
            Some(item)
        } else {
            None
        }
    }
}

pub struct EFSlice<'alloc, ID: EFID, T: 'static> {
    // The length of this slice is encoded in the reference itself (fat
    // pointer), and not located in / accessible to foreign memory:
    pub r: &'alloc [UnsafeCell<MaybeUninit<T>>],
    id_imprint: ID::Imprint,
}

impl<'alloc, ID: EFID, T: 'static> Clone for EFSlice<'alloc, ID, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'alloc, ID: EFID, T: 'static> Copy for EFSlice<'alloc, ID, T> {}

impl<'alloc, ID: EFID, T: 'static> EFSlice<'alloc, ID, T> {
    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }

    pub unsafe fn assume_valid<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> EFSliceVal<'alloc, 'access, ID, T> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        EFSliceVal {
            r: core::mem::transmute::<&[UnsafeCell<MaybeUninit<T>>], &[MaybeUninit<T>]>(&self.r),
            _id_imprint: self.id_imprint,
            _alloc_lt: PhantomData,
        }
    }

    pub fn as_ptr(&self) -> EFPtr<T> {
        EFPtr(self.r as *const _ as *mut [UnsafeCell<MaybeUninit<T>>] as *mut T)
    }

    pub fn len(&self) -> usize {
        self.r.len()
    }

    pub fn get(&self, idx: usize) -> Option<EFRef<'alloc, ID, T>> {
        self.r.get(idx).map(|elem| EFRef {
            r: elem,
            id_imprint: self.id_imprint,
        })
    }

    pub fn iter(&self) -> EFSliceIter<'alloc, ID, T> {
        EFSliceIter {
            inner: *self,
            idx: 0,
        }
    }
}

impl<'alloc, ID: EFID, T: EFType + 'static> EFSlice<'alloc, ID, T> {
    pub fn validate<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> Option<EFSliceVal<'alloc, 'access, ID, T>> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if DISABLE_VALIDATION_CHECKS {
            Some(unsafe { self.assume_valid(access_scope) })
        } else {
            if self
                .r
                .iter()
                .all(|elem: &UnsafeCell<MaybeUninit<T>>| unsafe {
                    <T as EFType>::validate(elem as *const UnsafeCell<MaybeUninit<T>> as *const T)
                })
            {
                Some(unsafe { self.assume_valid(access_scope) })
            } else {
                None
            }
        }
    }
}

impl<'alloc, ID: EFID> EFSlice<'alloc, ID, u8> {
    pub fn validate_as_str<'access>(
        &self,
        access_scope: &'access AccessScope<ID>,
    ) -> Option<EFVal<'alloc, 'access, ID, str>> {
        if self.id_imprint != access_scope.id_imprint() {
            panic!(
                "ID mismatch: {:?} vs. {:?}!",
                self.id_imprint,
                access_scope.id_imprint()
            );
        }

        if DISABLE_VALIDATION_CHECKS {
            Some(EFVal {
                r: unsafe { core::str::from_utf8_unchecked(&*(self.r as *const _ as *const [u8])) },
                id_imprint: self.id_imprint,
                _alloc_lt: PhantomData,
            })
        } else {
            // We rely on the fact that u8s are unconditionally valid, and we
            // hold onto an AccessScope here
            core::str::from_utf8(unsafe { &*(self.r as *const _ as *const [u8]) })
                .ok()
                .map(|s| EFVal {
                    r: s,
                    id_imprint: self.id_imprint,
                    _alloc_lt: PhantomData,
                })
        }
    }
}

pub struct EFSliceIter<'alloc, ID: EFID, T: 'static> {
    inner: EFSlice<'alloc, ID, T>,
    idx: usize,
}

impl<'alloc, ID: EFID, T: 'static> core::iter::Iterator for EFSliceIter<'alloc, ID, T> {
    type Item = EFRef<'alloc, ID, T>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.inner.get(self.idx) {
            // Prevent wraparound by calling .iter() a bunch.
            self.idx += 1;
            Some(item)
        } else {
            None
        }
    }
}

// -----------------------------------------------------------------------------

pub struct EFVal<'alloc, 'access, ID: EFID, T: 'static + ?Sized> {
    r: &'access T,
    id_imprint: ID::Imprint,
    _alloc_lt: PhantomData<&'alloc T>,
}

impl<'alloc, 'access, ID: EFID, T: 'static> EFVal<'alloc, 'access, ID, T> {
    pub fn id_imprint(&self) -> ID::Imprint {
        self.id_imprint
    }

    pub fn as_ref(&self) -> EFRef<'alloc, ID, T> {
        EFRef {
            r: unsafe { &*(self.r as *const _ as *const UnsafeCell<MaybeUninit<T>>) },
            id_imprint: self.id_imprint,
        }
    }

    pub fn as_mut(&self) -> EFMutRef<'alloc, ID, T> {
        EFMutRef {
            r: unsafe { &*(self.r as *const _ as *const UnsafeCell<MaybeUninit<T>>) },
            id_imprint: self.id_imprint,
        }
    }
}

impl<'alloc, 'access, ID: EFID, T: 'static + ?Sized> Deref for EFVal<'alloc, 'access, ID, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.r
    }
}

impl<'alloc, 'access, ID: EFID, T: 'static> Clone for EFVal<'alloc, 'access, ID, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'alloc, 'access, ID: EFID, T: 'static> Copy for EFVal<'alloc, 'access, ID, T> {}

impl<'alloc, 'access, const N: usize, ID: EFID, T> EFVal<'alloc, 'access, ID, [T; N]> {
    pub fn as_array(&self) -> &[EFVal<'alloc, 'access, ID, T>; N] {
        unsafe {
            core::mem::transmute::<
                &EFVal<'alloc, 'access, ID, [T; N]>,
                &[EFVal<'alloc, 'access, ID, T>; N],
            >(&self)
        }
    }
}

pub struct EFSliceVal<'alloc, 'access, ID: EFID, T: 'static> {
    r: &'access [MaybeUninit<T>],
    _id_imprint: ID::Imprint,
    _alloc_lt: PhantomData<&'alloc [T]>,
}

impl<'alloc, 'access, ID: EFID, T: EFType + 'static> Deref for EFSliceVal<'alloc, 'access, ID, T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        unsafe { core::mem::transmute::<&[MaybeUninit<T>], &[T]>(&self.r) }
    }
}

mod primitives {
    //! Implementations of [`EFType`] for primitive Rust types.

    use super::EFType;

    /// Validating an array requires validation of every element.
    unsafe impl<const N: usize, T: EFType> EFType for [T; N] {
        unsafe fn validate(array: *const Self) -> bool {
            // The array must have been validated to be well-aligned and
            // accessible. It must further be smaller than or equal to
            // isize::MAX in size, or otherwise the `.add` method invocation may
            // be unsound.
            //
            // This cast is important here. Otherwise we'd be stepping over the
            // array in its entirety and recursively calling this validate
            // function.
            let mut elem = array as *const T;

            // We'd like to use a Range<*mut T>.all() here, but `*mut T` does
            // not implement `core::iter::Step`...
            for _i in 0..N {
                if !EFType::validate(elem) {
                    // Abort on the first invalid element.
                    return false;
                }

                elem = elem.add(1);
            }

            // We iterated over the entire array and validated every single
            // element, to the entire array is valid:
            true
        }
    }

    macro_rules! unconditionally_valid {
	// Attempt to try to support generic arguments:
	// ($( #[ $attrs:tt ] )* for<$( $generics:ty ),*> $( $target:tt )*) => {
	//     $( #[ $attrs ] )*
	//     unsafe impl<$( $generics ),*> ::encapfn::types::EFType for $( $target )* {
	// 	fn validate(_t: *const Self) -> bool {
	// 	    // Unconditionally valid:
	// 	    true
	// 	}
	//     }
	// }

	// Non-generic:
	($( #[ $( $attrs:tt )* ] )* $target:ty) => {
	    /// Unconditionally valid type.
	    ///
	    /// As long as the memory backing this type is accessible,
	    /// well-aligned and conforms to Rust's aliasing requirements, we
	    /// can assume it to be valid without reading back its memory.
	    $( #[ $( $attrs )* ] )*
	    unsafe impl crate::types::EFType for $target {
		unsafe fn validate(_t: *const Self) -> bool {
		    // Unconditionally valid:
		    true
		}
	    }
	}
    }

    /// Accessing a raw pointer only requires that the pointer's numeric value
    /// is itself readable. Rust places no other restrictions on references to
    /// raw pointers. This does not mean that the resulting pointer is
    /// well-aligned, or safely dereferencable.
    unsafe impl<T> EFType for crate::types::EFPtr<T> {
        unsafe fn validate(_t: *const Self) -> bool {
            // Well-aligned and accessible pointer values are unconditionally
            // valid:
            true
        }
    }

    /// See the documentation for [`EFPtr as EFType`].
    unsafe impl<T> EFType for *const T {
        unsafe fn validate(_t: *const Self) -> bool {
            // Well-aligned and accessible pointer values are unconditionally
            // valid:
            true
        }
    }

    /// See the documentation for [`EFPtr as EFType`].
    unsafe impl<T> EFType for *mut T {
        unsafe fn validate(_t: *const Self) -> bool {
            // Well-aligned and accessible pointer values are unconditionally
            // valid:
            true
        }
    }

    // Implementations for primitives. We would like to implement these on the
    // `std::ffi::c_*` type aliases instead, but those are platform dependent
    // and may produce conflicting implementations. Hence we use Rust's
    // primitives, which the `std::ffi::c_*` type aliases point to, but for
    // which we can guarantee uniqueness:
    unconditionally_valid!(u8);
    unconditionally_valid!(u16);
    unconditionally_valid!(u32);
    unconditionally_valid!(u64);
    unconditionally_valid!(u128);
    unconditionally_valid!(usize);

    unconditionally_valid!(i8);
    unconditionally_valid!(i16);
    unconditionally_valid!(i32);
    unconditionally_valid!(i64);
    unconditionally_valid!(i128);
    unconditionally_valid!(isize);

    unconditionally_valid!(f32);
    unconditionally_valid!(f64);

    unconditionally_valid!(());

    /// See the documentation for [`EFPtr as EFType`].
    unsafe impl EFType for bool {
        unsafe fn validate(t: *const Self) -> bool {
            // Ensure that the integer type we load instead has an
            // equivalent layout:
            assert!(core::mem::size_of::<bool>() == core::mem::size_of::<u8>());
            assert!(core::mem::align_of::<bool>() == core::mem::align_of::<u8>());

            // Load the value as an integer and check that it is
            // within the range of valid boolean values:
            core::ptr::read(t as *const u8) < 2
        }
    }
}

/// Get an `EFMutRef` reference to a member of a struct wrapped in an
/// `EFMutRef`
///
/// TODO: this is a workaround until we derive EFType for nested types
/// in bindgen and provide safe methods for accessing struct members.
///
/// Usage example:
///
/// ```
/// use encapfn::types::EFMutRef;
/// use encapfn::branding::EFID;
/// use encapfn::efmutref_get_field;
///
/// struct TestStruct {
///     test_member: u32,
/// }
///
/// fn test_fn<'alloc, ID: EFID>(test_struct: EFMutRef<'alloc, ID, TestStruct>) {
///     let _test_member_ref: EFMutRef<'alloc, ID, u32> =
///         unsafe { efmutref_get_field!(TestStruct, u32, test_struct, test_member) };
/// }
/// ```
#[macro_export]
macro_rules! efmutref_get_field {
    ($outer_type:ty, $inner_type:ty, $outer_ref:expr, $member:ident) => {{
        unsafe fn efmutref_get_field_helper<'alloc, ID: $crate::branding::EFID>(
            outer: $crate::types::EFMutRef<'alloc, ID, $outer_type>,
        ) -> $crate::types::EFMutRef<'alloc, ID, $inner_type> {
            let outer_ptr: *mut () = outer.as_ptr().cast::<()>().into();
            let inner_efptr: $crate::types::EFPtr<$inner_type> =
                $crate::types::EFPtr::from(unsafe {
                    outer_ptr.byte_offset(::core::mem::offset_of!($outer_type, $member,) as isize)
                })
                .cast::<$inner_type>();
            unsafe { inner_efptr.upgrade_unchecked_mut(outer.id_imprint()) }
        }

        efmutref_get_field_helper($outer_ref)
    }};
}
