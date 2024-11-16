use core::cell::UnsafeCell;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::mem::MaybeUninit;

use crate::abi::GenericABI;
use crate::branding::EFID;
use crate::rt::EncapfnRt;
use crate::types::{AccessScope, AllocScope, AllocTracker, EFMutRef, EFPtr, EFRef, EFSlice};
use crate::EFError;

#[cfg_attr(feature = "nightly", doc(cfg(feature = "std")))]
#[cfg(any(feature = "std", doc))]
pub mod heap_alloc;

pub mod stack_alloc;

pub enum MockRtAllocError {
    InvalidLayout,
}

pub trait MockRtAllocator {
    unsafe fn with_alloc<R, F: FnOnce(*mut ()) -> R>(
        &self,
        layout: core::alloc::Layout,
        f: F,
    ) -> Result<R, MockRtAllocError>;
}

pub struct MockRt<ID: EFID, A: MockRtAllocator> {
    zero_copy_immutable: bool,
    allocator: A,
    _id: PhantomData<ID>,
}

impl<ID: EFID, A: MockRtAllocator> MockRt<ID, A> {
    pub unsafe fn new(
        zero_copy_immutable: bool,
        allocator: A,
        _branding: ID,
    ) -> (
        Self,
        AllocScope<'static, MockRtAllocTracker<'static>, ID>,
        AccessScope<ID>,
    ) {
        (
            MockRt {
                zero_copy_immutable,
                allocator,
                _id: PhantomData,
            },
            unsafe { AllocScope::new(MockRtAllocTracker(None)) },
            unsafe { AccessScope::new() },
        )
    }
}

#[derive(Clone, Debug)]
pub struct MockRtAllocTrackerCons<'a> {
    pred: &'a MockRtAllocTracker<'a>,
    allocation: (*mut (), usize),
    mutable: bool,
}

#[derive(Clone, Debug)]
pub struct MockRtAllocTracker<'a>(Option<MockRtAllocTrackerCons<'a>>);

impl MockRtAllocTracker<'_> {
    fn is_valid_int(&self, ptr: *mut (), len: usize, mutable: bool) -> bool {
        let mut cur = self;

        loop {
            if let Some(ref alloc) = cur.0 {
                let (aptr, alen) = alloc.allocation;

                // Make sure that:
                // - start address lies within region,
                // - end address lies within region,
                // - _if_ we require mutability, check that the allocation is
                //   mutable too.
                let matches = (ptr as usize) >= (aptr as usize)
                    && ((ptr as usize)
                        .checked_add(len)
                        .map(|end| end <= (aptr as usize) + alen)
                        .unwrap_or(false))
                    && (!mutable || alloc.mutable);

                if matches {
                    return true;
                } else {
                    cur = alloc.pred;
                }
            } else {
                return false;
            }
        }
    }
}

unsafe impl AllocTracker for MockRtAllocTracker<'_> {
    fn is_valid(&self, ptr: *const (), len: usize) -> bool {
        self.is_valid_int(ptr as *mut (), len, false)
    }

    fn is_valid_mut(&self, ptr: *mut (), len: usize) -> bool {
        self.is_valid_int(ptr, len, true)
    }
}

unsafe impl<ID: EFID, A: MockRtAllocator> EncapfnRt for MockRt<ID, A> {
    type ID = ID;
    type AllocTracker<'a> = MockRtAllocTracker<'a>;
    type ABI = GenericABI;

    type SymbolTableState<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize> = ();

    fn resolve_symbols<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize>(
        &self,
        _symbol_table: &'static [&'static CStr; SYMTAB_SIZE],
        _fixed_offset_symbol_table: &'static [Option<&'static CStr>; FIXED_OFFSET_SYMTAB_SIZE],
    ) -> Option<Self::SymbolTableState<SYMTAB_SIZE, FIXED_OFFSET_SYMTAB_SIZE>> {
        Some(())
    }

    fn lookup_symbol<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize>(
        &self,
        _compact_symtab_index: usize,
        _fixed_offset_symtab_index: usize,
        _symtabstate: &Self::SymbolTableState<SYMTAB_SIZE, FIXED_OFFSET_SYMTAB_SIZE>,
    ) -> Option<*const ()> {
        None
    }

    fn allocate_stacked_untracked_mut<F, R>(
        &self,
        layout: core::alloc::Layout,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: FnOnce(*mut ()) -> R,
    {
        // Simply proxy this to our underlying allocator:
        (unsafe { self.allocator.with_alloc(layout, fun) }).map_err(|e| match e {
            MockRtAllocError::InvalidLayout => EFError::AllocInvalidLayout,
        })
    }

    fn allocate_stacked_mut<F, R>(
        &self,
        layout: core::alloc::Layout,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(*mut (), &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>) -> R,
    {
        self.allocate_stacked_untracked_mut(layout, move |ptr| {
            // Create a new AllocScope instance that wraps a new allocation
            // tracker `Cons` list element that points to this allocation, and
            // its predecessors:
            let mut inner_alloc_scope = unsafe {
                AllocScope::new(MockRtAllocTracker(Some(MockRtAllocTrackerCons {
                    pred: alloc_scope.tracker(),
                    allocation: (ptr, layout.size()),
                    mutable: true,
                })))
            };

            // Hand a temporary mutable reference to this new scope to the
            // closure.
            //
            // We thus not only allocate, but also track allocations themselves
            // on the stack, and there is nothing to clean up! The new
            // `inner_alloc_scope` will simply go out of scope at the end of
            // this closure.
            fun(ptr, &mut inner_alloc_scope)
        })
    }

    fn allocate_stacked_t_mut<T: Sized + 'static, F, R>(
        &self,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutRef<'b, Self::ID, T>,
            &'b mut AllocScope<'b, Self::AllocTracker<'b>, Self::ID>,
        ) -> R,
    {
        let t = UnsafeCell::new(MaybeUninit::<T>::uninit());

        // Create a new AllocScope instance that wraps a new allocation
        // tracker `Cons` list element that points to this allocation, and
        // its predecessors:
        let mut inner_alloc_scope = unsafe {
            AllocScope::new(MockRtAllocTracker(Some(MockRtAllocTrackerCons {
                pred: alloc_scope.tracker(),
                allocation: (
                    &t as *const _ as *const _ as *mut _,
                    core::mem::size_of::<T>(),
                ),
                mutable: true,
            })))
        };

        // Hand a temporary mutable reference to this new scope to the
        // closure.
        //
        // We thus not only allocate, but also track allocations themselves
        // on the stack, and there is nothing to clean up! The new
        // `inner_alloc_scope` will simply go out of scope at the end of
        // this closure.
        Ok(fun(
            unsafe {
                EFPtr::<T>::from(&t as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T)
                    .upgrade_unchecked_mut()
            },
            &mut inner_alloc_scope,
        ))
    }

    fn write_stacked_t<T: Sized + 'static, F, R>(
        &self,
        t: T,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFRef<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        if self.zero_copy_immutable {
            // We can't wrap `write_stacked_ref_t` here, as our `T: ?Copy`.

            // While there are no guarantees that foreign code will uphold to
            // the immutability requirement with the MockRt, we still don't use
            // interior mutability here. This more closely simulates what a
            // proper runtime with memory protection would do.
            //
            // The soundness of this depends on whether the foreign code is
            // well-behaved, and whether the bindings correctly pass these
            // pointers *const arguments:
            let stored = t;

            // Create a new AllocScope instance that wraps a new allocation
            // tracker `Cons` list element that points to this allocation, and
            // its predecessors:
            let mut inner_alloc_scope = unsafe {
                AllocScope::new(MockRtAllocTracker(Some(MockRtAllocTrackerCons {
                    pred: alloc_scope.tracker(),
                    allocation: (
                        &stored as *const _ as *const _ as *mut _,
                        core::mem::size_of::<T>(),
                    ),
                    mutable: false,
                })))
            };

            // Hand a temporary immutable reference to this new scope to the
            // closure.
            //
            // We thus not only allocate, but also track allocations themselves
            // on the stack, and there is nothing to clean up! The new
            // `inner_alloc_scope` will simply go out of scope at the end of
            // this closure.
            Ok(fun(
                unsafe {
                    EFPtr::<T>::from(
                        &stored as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T,
                    )
                    .upgrade_unchecked()
                },
                &mut inner_alloc_scope,
                access_scope,
            ))
        } else {
            // Fall back onto default behavior:
            self.write_stacked_t_mut(
                t,
                alloc_scope,
                access_scope,
                |allocation, new_alloc_scope, new_access_scope| {
                    fun(allocation.as_immut(), new_alloc_scope, new_access_scope)
                },
            )
        }
    }

    fn write_stacked_ref_t<T: Sized + Copy + 'static, F, R>(
        &self,
        t: &T,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFRef<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        if self.zero_copy_immutable {
            // For safety considerations, see `write_stacked_t`.

            // Create a new AllocScope instance that wraps a new allocation
            // tracker `Cons` list element that points to this allocation, and
            // its predecessors:
            let mut inner_alloc_scope = unsafe {
                AllocScope::new(MockRtAllocTracker(Some(MockRtAllocTrackerCons {
                    pred: alloc_scope.tracker(),
                    allocation: (
                        t as *const _ as *const _ as *mut _,
                        core::mem::size_of::<T>(),
                    ),
                    mutable: false,
                })))
            };

            // Hand a temporary immutable reference to this new scope to the
            // closure.
            //
            // We thus not only allocate, but also track allocations themselves
            // on the stack, and there is nothing to clean up! The new
            // `inner_alloc_scope` will simply go out of scope at the end of
            // this closure.
            Ok(fun(
                unsafe {
                    EFPtr::<T>::from(t as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T)
                        .upgrade_unchecked()
                },
                &mut inner_alloc_scope,
                access_scope,
            ))
        } else {
            // Fall back onto default behavior:
            self.write_stacked_ref_t_mut(
                t,
                alloc_scope,
                access_scope,
                |allocation, new_alloc_scope, new_access_scope| {
                    fun(allocation.as_immut(), new_alloc_scope, new_access_scope)
                },
            )
        }
    }

    fn write_stacked_slice<T: Sized + Copy + 'static, F, R>(
        &self,
        src: &[T],
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFSlice<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        if self.zero_copy_immutable {
            // For safety considerations, see `write_stacked_t`.

            // Create a new AllocScope instance that wraps a new allocation
            // tracker `Cons` list element that points to this allocation, and
            // its predecessors:
            let mut inner_alloc_scope = unsafe {
                AllocScope::new(MockRtAllocTracker(Some(MockRtAllocTrackerCons {
                    pred: alloc_scope.tracker(),
                    allocation: (
                        src as *const _ as *const _ as *mut _,
                        core::mem::size_of::<T>() * src.len(),
                    ),
                    mutable: false,
                })))
            };

            // Hand a temporary immutable reference to this new scope to the
            // closure.
            //
            // We thus not only allocate, but also track allocations themselves
            // on the stack, and there is nothing to clean up! The new
            // `inner_alloc_scope` will simply go out of scope at the end of
            // this closure.
            Ok(fun(
                unsafe {
                    EFPtr::<T>::from(src as *const _ as *mut UnsafeCell<MaybeUninit<T>> as *mut T)
                        .upgrade_unchecked_slice(src.len())
                },
                &mut inner_alloc_scope,
                access_scope,
            ))
        } else {
            // Fall back onto default behavior:
            self.write_stacked_slice_from_iter(src.iter().copied(), alloc_scope, access_scope, fun)
        }
    }
}
