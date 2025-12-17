#[cfg(feature = "cnpg")]
pub mod operator {
    tonic::include_proto!("cnpgi.operator.v1");
}

#[cfg(feature = "cnpg")]
pub mod operator_lifecycle {
    tonic::include_proto!("cnpgi.operator_lifecycle.v1");
}

// Re-export commonly used types for convenience
#[cfg(feature = "cnpg")]
pub use operator_lifecycle::operator_lifecycle_server::{OperatorLifecycle, OperatorLifecycleServer};
#[cfg(feature = "cnpg")]
pub use operator_lifecycle::{OperatorLifecycleRequest, OperatorLifecycleResponse, OperatorOperationType};

#[cfg(feature = "cnpg")]
pub use operator::operator_server::{Operator, OperatorServer};
#[cfg(feature = "cnpg")]
pub use operator::{
    OperatorCapabilitiesRequest, OperatorCapabilitiesResult,
    OperatorMutateClusterRequest, OperatorMutateClusterResult,
    OperatorValidateClusterCreateRequest, OperatorValidateClusterCreateResult,
    OperatorValidateClusterChangeRequest, OperatorValidateClusterChangeResult,
    SetStatusInClusterRequest, SetStatusInClusterResponse,
    DeregisterRequest, DeregisterResponse,
    OperatorCapability, operator_capability::Rpc, operator_capability::rpc::Type as RpcType
};
