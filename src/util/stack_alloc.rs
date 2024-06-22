#[cfg(not(any(target_arch = "x86_64", target_arch = "riscv32",)))]
compile_error!("stack_alloc is not yet implemented for non-(x86_64|riscv64) platforms");

pub fn with_stacked_alloc<R, F: FnOnce(*mut ()) -> R>(size: usize, align: usize, f: F) -> R {
    enum Ret<RP> {
        Returned(RP),
        Unwinded,
    }

    struct Data<RP, FP> {
        // We'll need to move the closure out of here, as its an FnOnce
        closure: Option<FP>,
        ret: Ret<RP>,
    }

    unsafe extern "C" fn invoke<RP, FP: FnOnce(*mut ()) -> RP>(
        ptr: *mut (),
        _size: usize,
        data: *mut (),
    ) {
        let data: &mut Data<RP, FP> = unsafe { &mut *(data as *mut Data<RP, FP>) };

        // Assume that function doesn't unwind:
        let ret = (data.closure.take().unwrap())(ptr);
        data.ret = Ret::Returned(ret);
    }

    // Stack-allocate the context for the closure:
    let mut data = Data {
        // The callback will take() this closure ...
        closure: Some(f),
        // ... and it will set this value:
        ret: Ret::Unwinded,
    };

    // Now, run the closure, using a monomorphized version of our C-style
    // callback that knows the type of the closure and its return value (and
    // hence Data<R, F>, passing in the stacked context:
    unsafe {
        stack_alloc(
            size,
            align,
            invoke::<R, F>,
            &mut data as *mut Data<R, F> as *mut (),
        )
    };

    // Make sure that the closure has actually run:
    assert!(data.closure.is_none());

    // Finally, make sure the closure has run, has not unwinded, and return
    // the return value:
    match data.ret {
        // The function returned normally:
        Ret::Returned(ret) => return ret,

        // The function paniced, panic ourselves!
        Ret::Unwinded => panic!("with_stacked_alloc closure unwinded"),
    }
}

// We could potentially supply the function pointer as a generic argument, but
// this seems fine for now:
#[cfg(target_arch = "x86_64")]
unsafe fn stack_alloc(
    size: usize,
    align: usize,
    cb: unsafe extern "C" fn(*mut (), usize, *mut ()),
    data: *mut (),
) {
    // We only support power-of-two align, and align must be a positive value.
    assert!(align.is_power_of_two() && align >= 1);

    //// For x86-64, we actually want at least 8-byte alignment, such that we can
    //// efficiently push the new, saved stack pointer onto the stack:
    let align = core::cmp::max(16, align);

    // Calculate a bitmask that we can AND with the stack pointer to align it
    // downward:
    let align_bitmask = !align.wrapping_sub(1);

    // Magic:
    core::arch::asm!(
        "
        // Save the original stack pointer in a callee-saved register, as we
        // don't know ahead of time by how much we'll be moving it downward,
        // and need to restore it:
        mov r12, rsp

        // Move the stack pointer downward by `size`:
        sub rsp, rsi

        // We have allcated `size` bytes on the stack, but they may not be
        // properly aligned yet. We are given align_bitmask, which we can AND
        // with the stack pointer to align it downward efficiently.
        //
        // This is guaranteed to align our stack to a 16-byte boundary, as is
        // required for invoking our extern C function:
        and rsp, {align_bitmask_reg}

        // Now, call the function, with the allocated pointer (equal to rsp)
        // loaded in the first argument register:
        mov rdi, rsp
        call {cb_reg}

        // Finally, restore our old stack pointer:
        mov rsp, r12
        ",
        // Pass the second and third argument to our callback in the correct
        // registers already:
        in("si") size,
        in("dx") data,

        // Other values we need:
        cb_reg = in(reg) cb,
        align_bitmask_reg = in(reg) align_bitmask,

        // We additionally clobber r12 as a callee-saved register to store our
        // original stack pointer:
        out("r12") _,

        // Clobber all registers not preserved by a function call:
        clobber_abi("system"),
    );
}

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
unsafe fn stack_alloc(
    size: usize,
    align: usize,
    cb: unsafe extern "C" fn(*mut (), usize, *mut ()),
    data: *mut (),
) {
    // We only support power-of-two align, and align must be a positive value.
    assert!(align.is_power_of_two() && align >= 1);

    // For rv32 and rv64, we actually want at least 8-byte alignment, such
    // that we can efficiently push the new, saved stack pointer onto the stack:
    let align = core::cmp::max(16, align);

    // Calculate a bitmask that we can AND with the stack pointer to align it
    // downward:
    let align_bitmask = !align.wrapping_sub(1);

    // Magic:
    core::arch::asm!(
        "
        // Save the original stack pointer in a callee-saved register, as we
        // don't know ahead of time by how much we'll be moving it downward,
        // and need to restore it:
        mv s2, sp

        // Move the stack pointer downward by `size`:
        sub sp, sp, a1

        // We have allcated `size` bytes on the stack, but they may not be
        // properly aligned yet. We are given align_bitmask, which we can AND
        // with the stack pointer to align it downward efficiently.
        //
        // This is guaranteed to align our stack to a 16-byte boundary, as is
        // required for invoking our extern C function:
        and sp, sp, {align_bitmask_reg}

        // Now, call the function, with the allocated pointer (equal to sp)
        // loaded in the first argument register:
        mv a0, sp
        jalr {cb_reg}

        // Finally, restore our old stack pointer:
        mv sp, s2
        ",
        // Pass the second and third argument to our callback in the correct
        // registers already:
        in("a1") size,
        in("a2") data,

        // Other in(_) registers must not clobber a0:
        out("a0") _,

        // Other values we need:
        cb_reg = in(reg) cb,
        align_bitmask_reg = in(reg) align_bitmask,

        // We additionally clobber s2 as a callee-saved register to store our
        // original stack pointer:
        out("s2") _,

        // Clobber all registers not preserved by a function call:
        clobber_abi("system"),
    );
}
