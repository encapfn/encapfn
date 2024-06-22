pub trait EncapfnABI {}

pub mod calling_convention;
pub mod rv32i_c;
pub mod sysv_amd64;

// For Mock implementations, that don't have any ABI constraints
pub enum GenericABI {}
impl EncapfnABI for GenericABI {}
