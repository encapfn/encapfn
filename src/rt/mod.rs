pub mod mock;
pub mod rv32i_c;
pub mod sysv_amd64;

use crate::abi::EncapfnABI;
use crate::branding::EFID;
use crate::types::{
    AccessScope, AllocScope, AllocTracker, EFMutRef, EFMutSlice, EFPtr, EFRef, EFSlice,
};
use crate::EFError;

pub unsafe trait EncapfnRt {
    type ID: EFID;
    type AllocTracker<'a>: AllocTracker;
    type ABI: EncapfnABI;

    type SymbolTableState<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize>;

    fn resolve_symbols<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize>(
        &self,
        symbol_table: &'static [&'static core::ffi::CStr; SYMTAB_SIZE],
        fixed_offset_symbol_table: &'static [Option<&'static core::ffi::CStr>;
                     FIXED_OFFSET_SYMTAB_SIZE],
    ) -> Option<Self::SymbolTableState<SYMTAB_SIZE, FIXED_OFFSET_SYMTAB_SIZE>>;

    fn lookup_symbol<const SYMTAB_SIZE: usize, const FIXED_OFFSET_SYMTAB_SIZE: usize>(
        &self,
        index: usize,
        symtabstate: &Self::SymbolTableState<SYMTAB_SIZE, FIXED_OFFSET_SYMTAB_SIZE>,
    ) -> Option<*const ()>;

    // Can be used to set up memory protection before running the invoke asm.
    fn execute<R, F: FnOnce() -> R>(&self, f: F) -> R {
        // Default: nop
        f()
    }

    // TODO: document layout requirements!
    fn allocate_stacked_mut<F, R>(
        &self,
        layout: core::alloc::Layout,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(*mut (), &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>) -> R;

    fn allocate_stacked_untracked_mut<F, R>(
        &self,
        layout: core::alloc::Layout,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: FnOnce(*mut ()) -> R;

    // TODO: what about zero-sized T?
    fn allocate_stacked_t_mut<T: Sized + 'static, F, R>(
        &self,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutRef<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        ) -> R,
    {
        self.allocate_stacked_mut(
            core::alloc::Layout::new::<T>(),
            alloc_scope,
            |allocated_ptr, new_alloc_scope| {
                fun(
                    unsafe { EFPtr::<T>::from(allocated_ptr as *mut T).upgrade_unchecked_mut() },
                    new_alloc_scope,
                )
            },
        )
    }

    fn write_stacked_t_mut<T: Sized + 'static, F, R>(
        &self,
        t: T,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutRef<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        self.allocate_stacked_t_mut(alloc_scope, |allocation, new_alloc_scope| {
            allocation.write(t, access_scope);
            fun(allocation, new_alloc_scope, access_scope)
        })
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
        self.write_stacked_t_mut(
            t,
            alloc_scope,
            access_scope,
            |allocation, new_alloc_scope, new_access_scope| {
                fun(allocation.as_immut(), new_alloc_scope, new_access_scope)
            },
        )
    }

    fn write_stacked_ref_t_mut<T: Sized + Copy + 'static, F, R>(
        &self,
        t: &T,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutRef<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        self.allocate_stacked_t_mut(alloc_scope, |allocation, new_alloc_scope| {
            allocation.write_ref(t, access_scope);
            fun(allocation, new_alloc_scope, access_scope)
        })
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
        self.write_stacked_ref_t_mut(
            t,
            alloc_scope,
            access_scope,
            |allocation, new_alloc_scope, new_access_scope| {
                fun(allocation.as_immut(), new_alloc_scope, new_access_scope)
            },
        )
    }

    // TODO: what about zero-sized T?
    fn allocate_stacked_slice_mut<T: Sized + 'static, F, R>(
        &self,
        len: usize,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutSlice<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        ) -> R,
    {
        self.allocate_stacked_mut(
            core::alloc::Layout::array::<T>(len).unwrap(),
            alloc_scope,
            |allocated_ptr, new_alloc_scope| {
                fun(
                    unsafe {
                        EFPtr::<T>::from(allocated_ptr as *mut T).upgrade_unchecked_slice_mut(len)
                    },
                    new_alloc_scope,
                )
            },
        )
    }

    // TODO: what about an empty iterator?
    fn write_stacked_slice_from_iter_mut<T: Sized + 'static, F, R>(
        &self,
        src: impl Iterator<Item = T> + ExactSizeIterator,
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutSlice<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        self.allocate_stacked_slice_mut(src.len(), alloc_scope, |allocation, new_alloc_scope| {
            // This will panic if the iterator did not yield exactly `src.len()`
            // elements:
            allocation.write_from_iter(src, access_scope);
            fun(allocation, new_alloc_scope, access_scope)
        })
    }

    fn write_stacked_slice_mut<T: Sized + Copy + 'static, F, R>(
        &self,
        src: &[T],
        alloc_scope: &mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
        access_scope: &mut AccessScope<Self::ID>,
        fun: F,
    ) -> Result<R, EFError>
    where
        F: for<'b> FnOnce(
            EFMutSlice<'_, Self::ID, T>,
            &'b mut AllocScope<'_, Self::AllocTracker<'_>, Self::ID>,
            &'b mut AccessScope<Self::ID>,
        ) -> R,
    {
        self.write_stacked_slice_from_iter_mut(src.iter().copied(), alloc_scope, access_scope, fun)
    }

    fn write_stacked_slice_from_iter<T: Sized + 'static, F, R>(
        &self,
        src: impl Iterator<Item = T> + ExactSizeIterator,
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
        self.write_stacked_slice_from_iter_mut(
            src,
            alloc_scope,
            access_scope,
            |allocation, new_alloc_scope, new_access_scope| {
                fun(allocation.as_immut(), new_alloc_scope, new_access_scope)
            },
        )
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
        self.write_stacked_slice_from_iter(src.iter().copied(), alloc_scope, access_scope, fun)
    }
}
