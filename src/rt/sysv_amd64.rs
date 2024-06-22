use crate::rt::EncapfnRt;
use crate::EFResult;

pub unsafe trait SysVAMD64InvokeRes<RT: SysVAMD64BaseRt, T: Sized> {
    fn new() -> Self;

    fn into_result_registers(self, rt: &RT) -> EFResult<T>;
    unsafe fn into_result_stacked(self, rt: &RT, stacked_res: *mut T) -> EFResult<T>;
}

pub trait SysVAMD64BaseRt: EncapfnRt<ABI = crate::abi::sysv_amd64::SysVAMD64ABI> + Sized {
    type InvokeRes<T>: SysVAMD64InvokeRes<Self, T>;
}

pub trait SysVAMD64Rt<const STACK_SPILL: usize, RTLOC: crate::abi::calling_convention::ArgumentSlot>:
    SysVAMD64BaseRt
{
    unsafe extern "C" fn invoke();
}
