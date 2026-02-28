use std::fmt;
use std::sync::Arc;

use arrow::record_batch::RecordBatch;
use dbt_adapter::sidecar_client::{ColumnInfo, Connection, QueryCtx, SidecarClient};
use dbt_common::{AdapterError, AdapterResult};
use dbt_schemas::dbt_types::RelationType;
use minijinja::State;

use crate::circuit_breaker::CircuitBreaker;
use crate::controller::ChaosController;
use crate::fault::{Fault, TargetOperation};

/// A proxy `SidecarClient` that injects faults via `ChaosController`.
pub struct ChaosSidecarClient {
    inner: Arc<dyn SidecarClient>,
    controller: Arc<ChaosController>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
}

impl ChaosSidecarClient {
    pub fn new(
        inner: Arc<dyn SidecarClient>,
        controller: Arc<ChaosController>,
        circuit_breaker: Option<Arc<CircuitBreaker>>,
    ) -> Self {
        Self {
            inner,
            controller,
            circuit_breaker,
        }
    }

    fn intercept<T, F>(&self, operation: TargetOperation, f: F) -> AdapterResult<T>
    where
        F: FnOnce() -> AdapterResult<T>,
    {
        // Circuit breaker check
        if let Some(ref cb) = self.circuit_breaker {
            cb.pre_call()?;
        }

        // Chaos controller check
        if let Some(fault) = self.controller.maybe_inject(&operation) {
            let err = self.apply_fault(fault)?;
            // If apply_fault returns Ok, a latency fault was injected but we continue
            // to the inner call. If it returns Err, we propagate.
            if let Some(e) = err {
                if let Some(ref cb) = self.circuit_breaker {
                    cb.record_failure();
                }
                return Err(e);
            }
        }

        // Call through to inner
        match f() {
            Ok(result) => {
                if let Some(ref cb) = self.circuit_breaker {
                    cb.record_success();
                }
                Ok(result)
            }
            Err(e) => {
                if let Some(ref cb) = self.circuit_breaker {
                    cb.record_failure();
                }
                Err(e)
            }
        }
    }

    /// Apply a fault. Returns Ok(None) for latency-only faults (call should continue),
    /// or Ok(Some(error)) for faults that should abort the call.
    fn apply_fault(&self, fault: Fault) -> AdapterResult<Option<AdapterError>> {
        match fault {
            Fault::Latency(ref latency) => {
                self.controller.apply_latency(latency);
                Ok(None)
            }
            Fault::Error(ref error) => Ok(Some(ChaosController::fault_to_adapter_error(error))),
            Fault::PartialFailure(_) => {
                // Handled by controller; if we get here it's been converted to Error
                unreachable!("PartialFailure should be converted to Error by controller")
            }
            Fault::ConnectionDrop(ref drop) => {
                Ok(Some(ChaosController::connection_drop_to_adapter_error(drop)))
            }
            Fault::Timeout(ref timeout) => {
                Ok(Some(ChaosController::apply_timeout_fault(timeout)))
            }
        }
    }
}

impl fmt::Debug for ChaosSidecarClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChaosSidecarClient")
            .field("inner", &self.inner)
            .field("chaos_enabled", &self.controller.is_enabled())
            .finish()
    }
}

impl SidecarClient for ChaosSidecarClient {
    fn execute(
        &self,
        ctx: &QueryCtx,
        sql: &str,
        fetch: bool,
    ) -> AdapterResult<Option<RecordBatch>> {
        self.intercept(TargetOperation::Execute, || self.inner.execute(ctx, sql, fetch))
    }

    fn new_connection(
        &self,
        state: Option<&State>,
        node_id: Option<String>,
    ) -> AdapterResult<Box<dyn Connection>> {
        self.intercept(TargetOperation::NewConnection, || {
            self.inner.new_connection(state, node_id.clone())
        })
    }

    fn shutdown(&self) -> AdapterResult<()> {
        self.intercept(TargetOperation::Shutdown, || self.inner.shutdown())
    }

    fn get_relation_type(
        &self,
        schema: &str,
        table: &str,
    ) -> AdapterResult<Option<RelationType>> {
        self.intercept(TargetOperation::Metadata, || {
            self.inner.get_relation_type(schema, table)
        })
    }

    fn get_columns(&self, relation_name: &str) -> AdapterResult<Vec<ColumnInfo>> {
        self.intercept(TargetOperation::Metadata, || {
            self.inner.get_columns(relation_name)
        })
    }

    fn list_relations(
        &self,
        schema: &str,
    ) -> AdapterResult<Vec<(String, String, String, RelationType)>> {
        self.intercept(TargetOperation::Metadata, || {
            self.inner.list_relations(schema)
        })
    }
}
