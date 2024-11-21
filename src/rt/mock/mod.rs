use core::cell::UnsafeCell;
use core::ffi::{c_void, CStr};
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

// TODO: this should be a hashmap which takes a runtime ID derived from the EFID
// as key, to work with multiple mock runtimes in parallel:
static mut ACTIVE_ALLOC_CHAIN_HEAD_REF: Option<*const MockRtAllocChain<'static>> = None;

#[inline(never)]
extern "C" fn mock_rt_callback_dispatch<ID: EFID>(callback_id: usize) {
    let alloc_chain_head_ref_opt: &Option<*const MockRtAllocChain<'_>> =
        unsafe { &*core::ptr::addr_of!(ACTIVE_ALLOC_CHAIN_HEAD_REF) };

    let alloc_chain_head_ref_ptr = alloc_chain_head_ref_opt.unwrap();

    let alloc_chain_head_ref: &MockRtAllocChain<'static> = unsafe { &*alloc_chain_head_ref_ptr };

    unsafe { alloc_chain_head_ref.invoke_callback(callback_id) };
}

// TODO: reason about aliasing of the MockRtAllocChain
extern "C" fn mock_rt_callback_trampoline<const CALLBACK_ID: usize, ID: EFID>() {
    mock_rt_callback_dispatch::<ID>(CALLBACK_ID)
}

pub enum MockRtCallbackTrampolinePool<ID: EFID> {
    _Dummy(PhantomData<ID>, core::convert::Infallible),
}

impl<ID: EFID> MockRtCallbackTrampolinePool<ID> {
    // TODO: pre-generate trampolines with a macro
    const CALLBACKS: [extern "C" fn(); 32] = [
        mock_rt_callback_trampoline::<0, ID>,
        mock_rt_callback_trampoline::<1, ID>,
        mock_rt_callback_trampoline::<2, ID>,
        mock_rt_callback_trampoline::<3, ID>,
        mock_rt_callback_trampoline::<4, ID>,
        mock_rt_callback_trampoline::<5, ID>,
        mock_rt_callback_trampoline::<6, ID>,
        mock_rt_callback_trampoline::<7, ID>,
        mock_rt_callback_trampoline::<8, ID>,
        mock_rt_callback_trampoline::<9, ID>,
        mock_rt_callback_trampoline::<10, ID>,
        mock_rt_callback_trampoline::<11, ID>,
        mock_rt_callback_trampoline::<12, ID>,
        mock_rt_callback_trampoline::<13, ID>,
        mock_rt_callback_trampoline::<14, ID>,
        mock_rt_callback_trampoline::<15, ID>,
        mock_rt_callback_trampoline::<16, ID>,
        mock_rt_callback_trampoline::<17, ID>,
        mock_rt_callback_trampoline::<18, ID>,
        mock_rt_callback_trampoline::<19, ID>,
        mock_rt_callback_trampoline::<20, ID>,
        mock_rt_callback_trampoline::<21, ID>,
        mock_rt_callback_trampoline::<22, ID>,
        mock_rt_callback_trampoline::<23, ID>,
        mock_rt_callback_trampoline::<24, ID>,
        mock_rt_callback_trampoline::<25, ID>,
        mock_rt_callback_trampoline::<26, ID>,
        mock_rt_callback_trampoline::<27, ID>,
        mock_rt_callback_trampoline::<28, ID>,
        mock_rt_callback_trampoline::<29, ID>,
        mock_rt_callback_trampoline::<30, ID>,
        mock_rt_callback_trampoline::<31, ID>,
    ];
}

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
        AllocScope<'static, MockRtAllocChain<'static>, ID>,
        AccessScope<ID>,
    ) {
        (
            MockRt {
                zero_copy_immutable,
                allocator,
                _id: PhantomData,
            },
            unsafe { AllocScope::new(MockRtAllocChain::Base) },
            unsafe { AccessScope::new() },
        )
    }
}

#[derive(Clone, Debug)]
pub struct MockRtAllocation {
    ptr: *mut (),
    len: usize,
    mutable: bool,
}

impl MockRtAllocation {
    fn matches(&self, ptr: *mut (), len: usize, mutable: bool) -> bool {
        (ptr as usize) >= (self.ptr as usize)
            && ((ptr as usize)
                .checked_add(len)
                .map(|end| end <= (self.ptr as usize) + self.len)
                .unwrap_or(false))
            && (!mutable || self.mutable)
    }
}

#[derive(Debug)]
pub struct MockRtCallbackDescriptor<'a> {
    wrapper: unsafe extern "C-unwind" fn(*mut c_void),
    context: *mut c_void,
    _lt: PhantomData<&'a mut c_void>,
}

impl MockRtCallbackDescriptor<'_> {
    unsafe fn invoke(&self) {
        (self.wrapper)(self.context)
    }
}

#[derive(Debug)]
pub enum MockRtAllocChain<'a> {
    Base,
    Allocation(MockRtAllocation, &'a MockRtAllocChain<'a>),
    Callback(
        usize,
        MockRtCallbackDescriptor<'a>,
        &'a MockRtAllocChain<'a>,
    ),
}

struct MockRtAllocChainIter<'a>(Option<&'a MockRtAllocChain<'a>>);

impl<'a> Iterator for MockRtAllocChainIter<'a> {
    type Item = &'a MockRtAllocChain<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(cur) = self.0 {
            self.0 = match cur {
                MockRtAllocChain::Base => None,
                MockRtAllocChain::Allocation(_, pred) => Some(pred),
                MockRtAllocChain::Callback(_, _, pred) => Some(pred),
            };

            Some(cur)
        } else {
            None
        }
    }
}

impl<'a> MockRtAllocChain<'a> {
    fn iter(&'a self) -> MockRtAllocChainIter<'a> {
        MockRtAllocChainIter(Some(self))
    }

    fn is_valid_int(&self, ptr: *mut (), len: usize, mutable: bool) -> bool {
        self.iter().any(|elem| match elem {
            MockRtAllocChain::Base => false,
            MockRtAllocChain::Allocation(alloc, _) => alloc.matches(ptr, len, mutable),
            MockRtAllocChain::Callback(_, _, _) => false,
        })
    }

    fn next_callback_id(&self) -> usize {
        self.iter()
            .find_map(|elem| match elem {
                MockRtAllocChain::Base => None,
                MockRtAllocChain::Allocation(_, _) => None,
                MockRtAllocChain::Callback(id, _, _) => Some(id + 1),
            })
            .unwrap_or(0)
    }

    unsafe fn invoke_callback(&self, id: usize) {
        let callback_descriptor = self.iter().find_map(|elem| match elem {
            MockRtAllocChain::Base => None,
            MockRtAllocChain::Allocation(_, _) => None,
            MockRtAllocChain::Callback(desc_id, desc, _) => {
                if id == *desc_id {
                    Some(desc)
                } else {
                    None
                }
            }
        });

        if let Some(cb) = callback_descriptor {
            cb.invoke();
        } else {
            panic!("Callback descriptor with ID {:?} not found!", id);
        }
    }
}

unsafe impl AllocTracker for MockRtAllocChain<'_> {
    fn is_valid(&self, ptr: *const (), len: usize) -> bool {
        self.is_valid_int(ptr as *mut (), len, false)
    }

    fn is_valid_mut(&self, ptr: *mut (), len: usize) -> bool {
        self.is_valid_int(ptr, len, true)
    }
}

unsafe impl<ID: EFID, A: MockRtAllocator> EncapfnRt for MockRt<ID, A> {
    type ID = ID;
    type AllocTracker<'a> = MockRtAllocChain<'a>;
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

    fn setup_callback<'a, C, F, R>(
        &self,
        callback: &'a mut C,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        C: FnMut(),
        F: for<'b> FnOnce(
            *const extern "C" fn(),
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        ) -> R,
    {
        struct Context<'a, ClosureTy> {
            closure: &'a mut ClosureTy,
        }

        unsafe extern "C-unwind" fn callback_wrapper<'a, ClosureTy: FnMut() + 'a>(
            ctx_ptr: *mut c_void,
        ) {
            let ctx: &mut Context<'a, ClosureTy> =
                unsafe { &mut *(ctx_ptr as *mut Context<'a, ClosureTy>) };

            // For now, we assume that the functoin doesn't unwind:
            (ctx.closure)()
        }

        // Ensure that the context pointer is compatible in size and
        // layout to a c_void pointer:
        assert_eq!(
            core::mem::size_of::<*mut c_void>(),
            core::mem::size_of::<*mut Context<'a, C>>()
        );
        assert_eq!(
            core::mem::align_of::<*mut c_void>(),
            core::mem::align_of::<*mut Context<'a, C>>()
        );

        let mut ctx: Context<'a, C> = Context { closure: callback };

        let callback_id = alloc_scope.tracker().next_callback_id();

        let alloc_chain_head_ref_opt: &mut Option<*const MockRtAllocChain<'_>> =
            unsafe { &mut *core::ptr::addr_of_mut!(ACTIVE_ALLOC_CHAIN_HEAD_REF) };
        let outer_alloc_chain_head_ref = alloc_chain_head_ref_opt.clone();

        let mut inner_alloc_scope = unsafe {
            AllocScope::new(MockRtAllocChain::Callback(
                callback_id,
                MockRtCallbackDescriptor {
                    wrapper: callback_wrapper::<C>,
                    context: &mut ctx as *mut _ as *mut c_void,
                    _lt: PhantomData::<&'a mut c_void>,
                },
                alloc_scope.tracker(),
            ))
        };

        let tracker = inner_alloc_scope.tracker() as *const _;
        *alloc_chain_head_ref_opt = Some(tracker as *const MockRtAllocChain<'static>);
        let callback_trampoline = MockRtCallbackTrampolinePool::<ID>::CALLBACKS[callback_id];

        let res = fun(
            &callback_trampoline as *const extern "C" fn(),
            &mut inner_alloc_scope,
        );

        // Reset the alloc_chain_head_ref_opt:
        *alloc_chain_head_ref_opt = outer_alloc_chain_head_ref;

        Ok(res)
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
                AllocScope::new(MockRtAllocChain::Allocation(
                    MockRtAllocation {
                        ptr,
                        len: layout.size(),
                        mutable: true,
                    },
                    alloc_scope.tracker(),
                ))
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
            AllocScope::new(MockRtAllocChain::Allocation(
                MockRtAllocation {
                    ptr: &t as *const _ as *const _ as *mut _,
                    len: core::mem::size_of::<T>(),
                    mutable: true,
                },
                alloc_scope.tracker(),
            ))
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
                AllocScope::new(MockRtAllocChain::Allocation(
                    MockRtAllocation {
                        ptr: &stored as *const _ as *const _ as *mut _,
                        len: core::mem::size_of::<T>(),
                        mutable: false,
                    },
                    alloc_scope.tracker(),
                ))
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
                AllocScope::new(MockRtAllocChain::Allocation(
                    MockRtAllocation {
                        ptr: t as *const _ as *const _ as *mut _,
                        len: core::mem::size_of::<T>(),
                        mutable: false,
                    },
                    alloc_scope.tracker(),
                ))
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
                AllocScope::new(MockRtAllocChain::Allocation(
                    MockRtAllocation {
                        ptr: src as *const _ as *const _ as *mut _,
                        len: core::mem::size_of::<T>() * src.len(),
                        mutable: false,
                    },
                    alloc_scope.tracker(),
                ))
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
