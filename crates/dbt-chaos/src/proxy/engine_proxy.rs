use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::RecordBatch;
use dbt_adapter::adapter_engine::{AdapterEngine, Options};
use dbt_adapter::cache::RelationCache;
use dbt_adapter::config::AdapterConfig;
use dbt_adapter::query_cache::QueryCache;
use dbt_adapter::query_comment::QueryCommentConfig;
use dbt_adapter::sidecar_client::SidecarClient;
use dbt_adapter::sql_types::TypeOps;
use dbt_adapter::stmt_splitter::StmtSplitter;
use dbt_common::adapter::AdapterType;
use dbt_common::behavior_flags::Behavior;
use dbt_common::cancellation::CancellationToken;
use dbt_common::{AdapterError, AdapterResult};
use dbt_schemas::schemas::common::ResolvedQuoting;
use dbt_xdbc::{Backend, Connection, QueryCtx};
use minijinja::State;

use crate::circuit_breaker::CircuitBreaker;
use crate::controller::ChaosController;
use crate::fault::{Fault, TargetOperation};

/// A proxy `AdapterEngine` that injects faults via `ChaosController`.
pub struct ChaosEngine {
    inner: Arc<dyn AdapterEngine>,
    controller: Arc<ChaosController>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
}

impl ChaosEngine {
    pub fn new(
        inner: Arc<dyn AdapterEngine>,
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
        if let Some(fault) = self.controller.maybe_inject(&operation)
            && let Some(err) = self.apply_fault(fault)?
        {
            if let Some(ref cb) = self.circuit_breaker {
                cb.record_failure();
            }
            return Err(err);
        }

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

    fn apply_fault(&self, fault: Fault) -> AdapterResult<Option<AdapterError>> {
        match fault {
            Fault::Latency(ref latency) => {
                self.controller.apply_latency(latency);
                Ok(None)
            }
            Fault::Error(ref error) => Ok(Some(ChaosController::fault_to_adapter_error(error))),
            Fault::PartialFailure(_) => {
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

// -- Intercepted methods --

impl AdapterEngine for ChaosEngine {
    fn execute_with_options(
        &self,
        state: Option<&State>,
        ctx: &QueryCtx,
        conn: &'_ mut dyn Connection,
        sql: &str,
        options: Options,
        fetch: bool,
    ) -> AdapterResult<RecordBatch> {
        // Circuit breaker check
        if let Some(ref cb) = self.circuit_breaker {
            cb.pre_call()?;
        }

        // Chaos controller check
        if let Some(fault) = self.controller.maybe_inject(&TargetOperation::Execute)
            && let Some(err) = self.apply_fault(fault)?
        {
            if let Some(ref cb) = self.circuit_breaker {
                cb.record_failure();
            }
            return Err(err);
        }

        match self
            .inner
            .execute_with_options(state, ctx, conn, sql, options, fetch)
        {
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

    fn new_connection(
        &self,
        state: Option<&State>,
        node_id: Option<String>,
    ) -> AdapterResult<Box<dyn Connection>> {
        self.intercept(TargetOperation::NewConnection, || {
            self.inner.new_connection(state, node_id.clone())
        })
    }

    fn new_connection_with_config(
        &self,
        config: &AdapterConfig,
    ) -> AdapterResult<Box<dyn Connection>> {
        self.intercept(TargetOperation::NewConnection, || {
            self.inner.new_connection_with_config(config)
        })
    }

    // -- Pure delegation methods --

    fn adapter_type(&self) -> AdapterType {
        self.inner.adapter_type()
    }

    fn backend(&self) -> Backend {
        self.inner.backend()
    }

    fn quoting(&self) -> ResolvedQuoting {
        self.inner.quoting()
    }

    fn splitter(&self) -> &dyn StmtSplitter {
        self.inner.splitter()
    }

    fn type_ops(&self) -> &dyn TypeOps {
        self.inner.type_ops()
    }

    fn query_comment(&self) -> &QueryCommentConfig {
        self.inner.query_comment()
    }

    fn config(&self, key: &str) -> Option<Cow<'_, str>> {
        self.inner.config(key)
    }

    fn get_config(&self) -> &AdapterConfig {
        self.inner.get_config()
    }

    fn query_cache(&self) -> Option<&Arc<dyn QueryCache>> {
        self.inner.query_cache()
    }

    fn relation_cache(&self) -> &Arc<RelationCache> {
        self.inner.relation_cache()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.inner.cancellation_token()
    }

    fn behavior(&self) -> &Arc<Behavior> {
        self.inner.behavior()
    }

    fn behavior_flag_overrides(&self) -> &BTreeMap<String, bool> {
        self.inner.behavior_flag_overrides()
    }

    fn is_mock(&self) -> bool {
        self.inner.is_mock()
    }

    fn is_sidecar(&self) -> bool {
        self.inner.is_sidecar()
    }

    fn is_replay(&self) -> bool {
        self.inner.is_replay()
    }

    fn physical_backend(&self) -> Option<Backend> {
        self.inner.physical_backend()
    }

    fn sidecar_client(&self) -> Option<&dyn SidecarClient> {
        self.inner.sidecar_client()
    }
}
