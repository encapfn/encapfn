use crate::rt::EncapfnRt;
use crate::EFResult;

pub unsafe trait Rv32iCInvokeRes<RT: Rv32iCBaseRt, T: Sized> {
    fn new() -> Self;

    fn into_result_registers(self, rt: &RT) -> EFResult<T>;
    unsafe fn into_result_stacked(self, rt: &RT, stacked_res: *mut T) -> EFResult<T>;
}

pub trait Rv32iCBaseRt: EncapfnRt<ABI = crate::abi::rv32i_c::Rv32iCABI> + Sized {
    type InvokeRes<T>: Rv32iCInvokeRes<Self, T>;
}

pub trait Rv32iCRt<const STACK_SPILL: usize, RTLOC: crate::abi::calling_convention::ArgumentSlot>:
    Rv32iCBaseRt
{
    unsafe extern "C" fn invoke();
}
