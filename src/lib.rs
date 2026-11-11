#![no_std]
#![doc = include_str!("../README.md")]

mod capabilities;
mod command;
mod completion;
mod configuration;
mod doorbell;
mod identify;
mod namespace;
mod physical_region_page;
mod queues;
pub mod registers;

#[cfg(kani)]
mod verification;

pub use capabilities::Capabilities;
pub use command::{
    ADMIN_CREATE_IO_COMPLETION_QUEUE, ADMIN_CREATE_IO_SUBMISSION_QUEUE, ADMIN_IDENTIFY,
    CONTROLLER_OR_NAMESPACE_STRUCTURE_ACTIVE_NAMESPACE_LIST,
    CONTROLLER_OR_NAMESPACE_STRUCTURE_CONTROLLER, CONTROLLER_OR_NAMESPACE_STRUCTURE_NAMESPACE,
    Command, NON_VOLATILE_MEMORY_FLUSH, NON_VOLATILE_MEMORY_READ, NON_VOLATILE_MEMORY_WRITE,
};
pub use completion::{
    Completion, GENERIC_STATUS_INVALID_FIELD_IN_COMMAND, GENERIC_STATUS_SUCCESSFUL_COMPLETION,
    STATUS_CODE_TYPE_GENERIC,
};
pub use configuration::{
    admin_queue_attributes, enabled_controller_configuration, shutdown_controller_configuration,
};
pub use doorbell::{Doorbell, doorbell};
pub use identify::{
    IdentifyController, IdentifyNamespace, logical_block_size_shift, parse_identify_controller,
    parse_identify_namespace,
};
pub use namespace::{Direction, Namespace};
pub use physical_region_page::{PhysicalRegionPagePair, physical_region_page_pair};
pub use queues::{CompletionQueue, ImpossibleHeadError, SubmissionQueue};
pub use registers::{
    CONTROLLER_STATUS_FATAL, CONTROLLER_STATUS_READY, CONTROLLER_STATUS_SHUTDOWN_COMPLETE,
    CONTROLLER_STATUS_SHUTDOWN_OCCURRING,
};
