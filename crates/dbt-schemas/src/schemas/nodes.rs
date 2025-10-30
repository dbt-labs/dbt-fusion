use core::fmt;
use std::str::FromStr;
use std::{any::Any, collections::BTreeMap, fmt::Display, path::PathBuf, sync::Arc};

use chrono::{DateTime, Utc};
use dbt_common::adapter::AdapterType;
use dbt_common::io_args::StaticAnalysisOffReason;
use dbt_common::{ErrorCode, FsResult, err, io_args::StaticAnalysisKind};
use dbt_telemetry::{ExecutionPhase, NodeEvaluated, NodeType};
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
type YmlValue = dbt_serde_yaml::Value;
use crate::schemas::common::{NodeInfo, NodeInfoWrapper, PersistDocsConfig};
use crate::schemas::dbt_column::{DbtColumnRef, deserialize_dbt_columns, serialize_dbt_columns};
use crate::schemas::manifest::{BigqueryClusterConfig, GrantAccessToTarget, PartitionConfig};
use crate::schemas::project::WarehouseSpecificNodeConfig;
use crate::schemas::serde::StringOrArrayOfStrings;
use crate::schemas::{
    common::{
        Access, DbtChecksum, DbtContract, DbtIncrementalStrategy, DbtMaterialization, Expect,
        FreshnessDefinition, Given, IncludeExclude, NodeDependsOn, ResolvedQuoting, ScheduleConfig,
    },
    macros::DbtMacro,
    manifest::common::DbtOwner,
    manifest::semantic_model::NodeRelation,
    manifest::{DbtMetric, DbtSavedQuery, DbtSemanticModel},
    project::{
        DataTestConfig, ExposureConfig, FunctionConfig, ModelConfig, SeedConfig, SnapshotConfig,
        SnapshotMetaColumnNames, SourceConfig, UnitTestConfig,
    },
    properties::{
        FunctionArgument, FunctionKind, FunctionReturnType, ModelConstraint, ModelFreshness,
        UnitTestOverrides,
    },
    ref_and_source::{DbtRef, DbtSourceWrapper},
    serde::StringOrInteger,
};
use dbt_serde_yaml::{Spanned, UntaggedEnumDeserialize};

#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum IntrospectionKind {
    #[default]
    None,
    Execute,
    This,
    UpstreamSchema,
    InternalSchema,
    ExternalSchema,
    Unknown,
}

impl IntrospectionKind {
    pub fn is_unsafe(&self) -> bool {
        matches!(
            self,
            IntrospectionKind::Execute
                | IntrospectionKind::InternalSchema
                | IntrospectionKind::ExternalSchema
                | IntrospectionKind::This
                | IntrospectionKind::Unknown
        )
    }
}

impl Display for IntrospectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntrospectionKind::None => write!(f, "none"),
            IntrospectionKind::Execute => write!(f, "execute"),
            IntrospectionKind::UpstreamSchema => write!(f, "upstream_schema"),
            IntrospectionKind::InternalSchema => write!(f, "internal_schema"),
            IntrospectionKind::ExternalSchema => write!(f, "external_schema"),
            IntrospectionKind::This => write!(f, "this"),
            IntrospectionKind::Unknown => write!(f, "unknown"),
        }
    }
}

impl FromStr for IntrospectionKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(IntrospectionKind::None),
            "execute" => Ok(IntrospectionKind::Execute),
            "upstream_schema" => Ok(IntrospectionKind::UpstreamSchema),
            "internal_schema" => Ok(IntrospectionKind::InternalSchema),
            "external_schema" => Ok(IntrospectionKind::ExternalSchema),
            "this" => Ok(IntrospectionKind::This),
            _ => Err(()),
        }
    }
}

/// A wrapper enum that represents different types of dbt nodes.
///
/// This enum uses serde's tag-based deserialization to automatically determine
/// the correct variant based on the "resource_type" field in the JSON.
/// The resource_type values are converted to snake_case for matching.
///
/// # Example
///
/// ```rust
///
/// // Deserialize a node from Jinja
/// let node = minijinja_value_to_typed_struct::<InternalDbtNodeWrapper>(value).unwrap();
///
/// // Access the underlying node attributes
/// let attributes = node.as_internal_node();
/// ```
#[derive(Debug, Clone, Serialize, UntaggedEnumDeserialize)]
#[serde(tag = "resource_type")]
#[serde(rename_all = "snake_case")]
pub enum InternalDbtNodeWrapper {
    Model(Box<DbtModel>),
    Seed(Box<DbtSeed>),
    Test(Box<DbtTest>),
    UnitTest(Box<DbtUnitTest>),
    Source(Box<DbtSource>),
    Snapshot(Box<DbtSnapshot>),
    Exposure(Box<DbtExposure>),
    Function(Box<DbtFunction>),
}

impl InternalDbtNodeWrapper {
    /// Returns a reference to the underlying node as a trait object.
    ///
    /// This method allows accessing common functionality across all node types
    /// through the `InternalDbtNodeAttributes` trait.
    ///
    /// # Returns
    ///
    /// A reference to the node implementing `InternalDbtNodeAttributes`
    ///
    /// # Examples
    ///
    /// ```rust
    /// let node = InternalDbtNodeWrapper::Model(some_model);
    /// let attributes = node.as_internal_node();
    /// println!("Node name: {}", attributes.name());
    /// ```
    pub fn as_internal_node(&self) -> &dyn InternalDbtNodeAttributes {
        match self {
            InternalDbtNodeWrapper::Model(model) => model.as_ref(),
            InternalDbtNodeWrapper::Seed(seed) => seed.as_ref(),
            InternalDbtNodeWrapper::Test(test) => test.as_ref(),
            InternalDbtNodeWrapper::UnitTest(unit_test) => unit_test.as_ref(),
            InternalDbtNodeWrapper::Source(source) => source.as_ref(),
            InternalDbtNodeWrapper::Snapshot(snapshot) => snapshot.as_ref(),
            InternalDbtNodeWrapper::Exposure(exposure) => exposure.as_ref(),
            InternalDbtNodeWrapper::Function(function) => function.as_ref(),
        }
    }
}

pub trait InternalDbtNode: Any + Send + Sync + fmt::Debug {
    fn common(&self) -> &CommonAttributes;
    fn base(&self) -> &NodeBaseAttributes;
    fn base_mut(&mut self) -> &mut NodeBaseAttributes;
    fn common_mut(&mut self) -> &mut CommonAttributes;
    fn version(&self) -> Option<StringOrInteger> {
        None
    }
    fn latest_version(&self) -> Option<StringOrInteger> {
        None
    }
    fn event_time(&self) -> Option<String> {
        None
    }
    fn is_extended_model(&self) -> bool {
        false
    }
    fn is_versioned(&self) -> bool {
        false
    }
    fn defined_at(&self) -> Option<&dbt_common::CodeLocation> {
        None
    }
    fn resource_type(&self) -> NodeType;
    fn as_any(&self) -> &dyn Any;
    fn serialize(&self) -> YmlValue {
        let mut ret = self.serialize_inner();
        if let YmlValue::Mapping(ref mut map, _) = ret {
            map.insert(
                YmlValue::string("resource_type".to_string()),
                YmlValue::string(self.resource_type().as_ref().to_string()),
            );
        }
        ret
    }
    fn serialize_inner(&self) -> YmlValue;

    // Selector functions
    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool;
    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool;
    fn set_detected_introspection(&mut self, introspection: IntrospectionKind);
    fn introspection(&self) -> IntrospectionKind {
        IntrospectionKind::None
    }

    fn is_test(&self) -> bool {
        self.resource_type() == NodeType::Test
    }

    // Incremental strategy validation
    fn warn_on_microbatch(&self) -> FsResult<()> {
        Ok(())
    }

    fn get_node_start_data(&self, node_started_at: DateTime<Utc>) -> NodeInfoWrapper {
        NodeInfoWrapper {
            unique_id: None,
            skipped_nodes: None,
            defined_at: self.defined_at().cloned(),
            node_info: NodeInfo {
                node_name: self.common().name.clone(),
                unique_id: self.common().unique_id.clone(),
                node_started_at: Some(node_started_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()),
                node_finished_at: None,
                node_status: "executing".to_string(),
            },
        }
    }

    fn get_node_end_data(
        &self,
        status: &str,
        node_started_at: DateTime<Utc>,
        node_finished_at: DateTime<Utc>,
    ) -> NodeInfoWrapper {
        NodeInfoWrapper {
            unique_id: None,
            skipped_nodes: None,
            defined_at: self.defined_at().cloned(),
            node_info: NodeInfo {
                node_name: self.common().name.clone(),
                unique_id: self.common().unique_id.clone(),
                node_started_at: Some(node_started_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string()),
                node_finished_at: Some(
                    node_finished_at.format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
                ),
                node_status: status.to_string(),
            },
        }
    }

    fn get_node_evaluated_event(&self, phase: ExecutionPhase) -> NodeEvaluated {
        node_evaluated_event_from_attrs(self.common(), self.base(), self.resource_type(), phase)
    }
}

pub fn node_evaluated_event_from_attrs(
    common: &CommonAttributes,
    base: &NodeBaseAttributes,
    node_type: NodeType,
    phase: ExecutionPhase,
) -> NodeEvaluated {
    let (database, schema, identifier) = (
        base.database.clone(),
        base.schema.clone(),
        Some(base.alias.clone()),
    );

    let custom_materialization = if let DbtMaterialization::Unknown(custom) = &base.materialized {
        Some(custom.clone())
    } else {
        None
    };

    NodeEvaluated::start(
        common.unique_id.clone(),
        common.name.clone(),
        Some(database),
        Some(schema),
        identifier,
        Some((&base.materialized).into()),
        custom_materialization,
        node_type,
        phase,
    )
}

pub trait InternalDbtNodeAttributes: InternalDbtNode {
    // Required Fields
    fn skip_generate_database_name_macro(&self) -> bool {
        false
    }

    fn database(&self) -> String {
        self.base().database.clone()
    }

    fn skip_generate_schema_name_macro(&self) -> bool {
        false
    }

    fn schema(&self) -> String {
        self.base().schema.clone()
    }

    fn unique_id(&self) -> String {
        self.common().unique_id.clone()
    }

    fn name(&self) -> String {
        self.common().name.clone()
    }

    fn alias(&self) -> String {
        self.base().alias.clone()
    }

    fn path(&self) -> PathBuf {
        self.common().path.clone()
    }

    fn original_file_path(&self) -> PathBuf {
        self.common().original_file_path.clone()
    }

    fn package_name(&self) -> String {
        self.common().package_name.clone()
    }

    fn materialized(&self) -> DbtMaterialization {
        self.base().materialized.clone()
    }

    fn quoting(&self) -> ResolvedQuoting {
        self.base().quoting
    }

    fn tags(&self) -> Vec<String> {
        self.common().tags.clone()
    }

    fn meta(&self) -> BTreeMap<String, YmlValue> {
        self.common().meta.clone()
    }

    fn static_analysis(&self) -> Spanned<StaticAnalysisKind> {
        self.base().static_analysis.clone()
    }

    fn static_analysis_enabled(&self) -> Spanned<bool> {
        self.static_analysis().map(|static_analysis| {
            static_analysis == StaticAnalysisKind::On
                || static_analysis == StaticAnalysisKind::Unsafe
        })
    }

    fn static_analysis_off_reason(&self) -> Option<StaticAnalysisOffReason> {
        self.base().static_analysis_off_reason
    }
    // Setters

    fn set_quoting(&mut self, quoting: ResolvedQuoting) {
        self.base_mut().quoting = quoting;
    }

    fn set_static_analysis(&mut self, static_analysis: Spanned<StaticAnalysisKind>) {
        self.base_mut().static_analysis = static_analysis;
    }

    fn set_static_analysis_off_reason(
        &mut self,
        static_analysis_off_reason: Option<StaticAnalysisOffReason>,
    ) {
        self.base_mut().static_analysis_off_reason = static_analysis_off_reason;
    }

    // Optional Fields
    fn get_access(&self) -> Option<Access> {
        None
    }
    fn get_group(&self) -> Option<String> {
        None
    }

    /// Returns the search name for this node, following Python dbt patterns:
    /// - Models: name (or name.v{version} if versioned)
    /// - Sources: source_name.name
    /// - Others: name
    fn search_name(&self) -> String;

    /// Returns the selector string for this node, following Python dbt patterns:
    /// - Models/Seeds/Tests/Snapshots: use fqn joined with "."
    /// - Sources: "source:pkg.source_name.table_name"
    /// - Unit tests: "unit_test:pkg.versioned_name"
    fn selector_string(&self) -> String;

    /// Returns the file path for this node
    fn file_path(&self) -> String {
        self.common()
            .original_file_path
            .to_string_lossy()
            .to_string()
    }

    // TO BE DEPRECATED
    fn serialized_config(&self) -> YmlValue;
}

impl InternalDbtNode for DbtModel {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn version(&self) -> Option<StringOrInteger> {
        self.__model_attr__.version.clone()
    }

    fn latest_version(&self) -> Option<StringOrInteger> {
        self.__model_attr__.latest_version.clone()
    }

    fn event_time(&self) -> Option<String> {
        self.__model_attr__.event_time.clone()
    }

    fn is_versioned(&self) -> bool {
        self.__model_attr__.version.is_some()
    }

    fn is_extended_model(&self) -> bool {
        self.__base_attr__.extended_model
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Model
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_model) = other.as_any().downcast_ref::<DbtModel>() {
            self.deprecated_config == other_model.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        // TODO: the checksum for extended model is always different in mantle and fusion, dig more into this
        if self.is_extended_model() {
            return true;
        }
        if let Some(other_model) = other.as_any().downcast_ref::<DbtModel>() {
            self.__common_attr__.checksum == other_model.__common_attr__.checksum
        } else {
            false
        }
    }
    fn set_detected_introspection(&mut self, introspection: IntrospectionKind) {
        self.__model_attr__.introspection = introspection;
    }
    fn introspection(&self) -> IntrospectionKind {
        self.__model_attr__.introspection
    }

    fn warn_on_microbatch(&self) -> FsResult<()> {
        if let Some(DbtIncrementalStrategy::Microbatch) = self.__model_attr__.incremental_strategy {
            return err!(
                code => ErrorCode::UnsupportedFeature,
                loc => self.path(),
                "Microbatch incremental strategy is not supported. Use --exclude config.incremental_strategy:microbatch to exclude these models."
            );
        }
        Ok(())
    }
}

impl InternalDbtNodeAttributes for DbtModel {
    fn get_access(&self) -> Option<Access> {
        Some(self.__model_attr__.access.clone())
    }

    fn get_group(&self) -> Option<String> {
        self.__model_attr__.group.clone()
    }

    fn search_name(&self) -> String {
        if let Some(version) = &self.__model_attr__.version {
            format!("{}.v{}", self.__common_attr__.name, version)
        } else {
            self.__common_attr__.name.clone()
        }
    }

    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtSeed {
    fn resource_type(&self) -> NodeType {
        NodeType::Seed
    }

    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_model) = other.as_any().downcast_ref::<DbtSeed>() {
            self.deprecated_config == other_model.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        //TODO: the checksum for seed is different between mantle and fusion.
        true
        // if let Some(other_model) = other.as_any().downcast_ref::<DbtSeed>() {
        //     self.common_attr.checksum == other_model.common_attr.checksum
        // } else {
        //     false
        // }
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtSeed does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtSeed {
    fn set_static_analysis(&mut self, _static_analysis: Spanned<StaticAnalysisKind>) {
        unimplemented!("static analysis metadata setting for schema nodes")
    }

    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }

    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtTest {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Test
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn defined_at(&self) -> Option<&dbt_common::CodeLocation> {
        self.defined_at.as_ref()
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<DbtTest>() {
            // these fields are what dbt compares for test nodes
            // Some other configs were skipped
            self.deprecated_config.enabled == other.deprecated_config.enabled
                && self.deprecated_config.alias == other.deprecated_config.alias
                && self.deprecated_config.database == other.deprecated_config.database
                && self.deprecated_config.tags == other.deprecated_config.tags
                && self.deprecated_config.meta == other.deprecated_config.meta
                && self.deprecated_config.group == other.deprecated_config.group
                && self.deprecated_config.quoting == other.deprecated_config.quoting
        } else {
            false
        }
    }

    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        // TODO: test currently is not supported for state selector due to the difference of test name generation between fusion and dbt-mantle.
        true
    }
    fn set_detected_introspection(&mut self, introspection: IntrospectionKind) {
        self.__test_attr__.introspection = introspection;
    }
    fn introspection(&self) -> IntrospectionKind {
        self.__test_attr__.introspection
    }
}

impl InternalDbtNodeAttributes for DbtTest {
    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }

    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtUnitTest {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::UnitTest
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<DbtUnitTest>() {
            self.deprecated_config == other.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other) = other.as_any().downcast_ref::<DbtUnitTest>() {
            let same_fqn = self.common().fqn == other.common().fqn;
            if !same_fqn {
                println!(
                    "FQN differs: self={:?}, other={:?}",
                    self.common().fqn,
                    other.common().fqn
                );
            }
            same_fqn
        } else {
            false
        }
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtUnitTest does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtUnitTest {
    fn search_name(&self) -> String {
        // Based on Python implementation, unit tests can have a versioned name
        if let Some(version) = &self.__unit_test_attr__.version {
            format!("{}_v{}", self.__common_attr__.name, version)
        } else {
            self.__common_attr__.name.clone()
        }
    }

    fn selector_string(&self) -> String {
        format!(
            "unit_test:{}.{}",
            self.__common_attr__.package_name,
            self.search_name()
        )
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtSource {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Source
    }

    fn event_time(&self) -> Option<String> {
        self.deprecated_config.event_time.clone()
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_source) = other.as_any().downcast_ref::<DbtSource>() {
            self.deprecated_config == other_source.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_source) = other.as_any().downcast_ref::<DbtSource>() {
            // Relation name capture database, schema and identifier
            self.__base_attr__.relation_name == other_source.__base_attr__.relation_name
                && self.__common_attr__.fqn == other_source.__common_attr__.fqn
                //TODO: uncomment this when we have a way to compare the config
                // && self.deprecated_config == other_source.deprecated_config
                // && self.base_attr.quoting == other_source.base_attr.quoting
                && self.__source_attr__.loader == other_source.__source_attr__.loader
        } else {
            false
        }
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtSource does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtSource {
    fn search_name(&self) -> String {
        format!(
            "{}.{}",
            self.__source_attr__.source_name, self.__common_attr__.name
        )
    }

    fn selector_string(&self) -> String {
        format!(
            "source:{}.{}.{}",
            self.__common_attr__.package_name,
            self.__source_attr__.source_name,
            self.__common_attr__.name
        )
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize DbtModel")
    }
}

impl InternalDbtNode for DbtSnapshot {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Snapshot
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_snapshot) = other.as_any().downcast_ref::<DbtSnapshot>() {
            self.deprecated_config == other_snapshot.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        // TODO: support snapshot state comparison by generate the same hash.
        true
        // if let Some(other_snapshot) = other.as_any().downcast_ref::<DbtSnapshot>() {
        //     self.common_attr.checksum == other_snapshot.common_attr.checksum
        // } else {
        //     false
        // }
    }

    fn set_detected_introspection(&mut self, introspection: IntrospectionKind) {
        self.__snapshot_attr__.introspection = introspection;
    }

    fn introspection(&self) -> IntrospectionKind {
        self.__snapshot_attr__.introspection
    }
}

impl InternalDbtNodeAttributes for DbtSnapshot {
    fn skip_generate_schema_name_macro(&self) -> bool {
        self.deprecated_config.target_schema.is_some()
    }

    fn schema(&self) -> String {
        // prefer legacy config
        self.deprecated_config
            .target_schema
            .clone()
            .unwrap_or_else(|| self.base().schema.clone())
    }

    fn skip_generate_database_name_macro(&self) -> bool {
        self.deprecated_config.target_database.is_some()
    }

    fn database(&self) -> String {
        // prefer legacy config
        self.deprecated_config
            .target_database
            .clone()
            .unwrap_or_else(|| self.base().database.clone())
    }

    fn tags(&self) -> Vec<String> {
        self.__common_attr__.tags.clone()
    }

    fn meta(&self) -> BTreeMap<String, YmlValue> {
        self.__common_attr__.meta.clone()
    }

    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }

    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtExposure {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }
    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }
    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }
    fn resource_type(&self) -> NodeType {
        NodeType::Exposure
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }
    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_exposure) = other.as_any().downcast_ref::<DbtExposure>() {
            self.deprecated_config == other_exposure.deprecated_config
        } else {
            false
        }
    }
    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_exposure) = other.as_any().downcast_ref::<DbtExposure>() {
            self.__common_attr__.name == other_exposure.__common_attr__.name
                && self.__common_attr__.fqn == other_exposure.__common_attr__.fqn
        } else {
            false
        }
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtExposure does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtExposure {
    fn materialized(&self) -> DbtMaterialization {
        self.__base_attr__.materialized.clone()
    }
    fn quoting(&self) -> ResolvedQuoting {
        self.__base_attr__.quoting
    }
    fn tags(&self) -> Vec<String> {
        self.__common_attr__.tags.clone()
    }
    fn meta(&self) -> BTreeMap<String, YmlValue> {
        self.__common_attr__.meta.clone()
    }
    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }
    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }
}

impl InternalDbtNode for DbtSemanticModel {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }
    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }
    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }
    fn resource_type(&self) -> NodeType {
        NodeType::SemanticModel
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }
    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(_other_semantic_model) = other.as_any().downcast_ref::<DbtSemanticModel>() {
            // TODO: implement proper config comparison when needed
            true
        } else {
            false
        }
    }
    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        unimplemented!("semantic model content comparison")
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtSemanticModel does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtSemanticModel {
    fn set_quoting(&mut self, _quoting: ResolvedQuoting) {
        unimplemented!("")
    }
    fn set_static_analysis(&mut self, _static_analysis: Spanned<StaticAnalysisKind>) {
        unimplemented!("")
    }
    fn search_name(&self) -> String {
        self.name()
    }
    fn selector_string(&self) -> String {
        format!(
            "semantic_model:{}.{}",
            self.package_name(),
            self.search_name()
        )
    }
    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtMetric {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }
    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }
    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }
    fn resource_type(&self) -> NodeType {
        NodeType::Metric
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }
    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_metric) = other.as_any().downcast_ref::<DbtMetric>() {
            self.deprecated_config == other_metric.deprecated_config
        } else {
            false
        }
    }
    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_metric) = other.as_any().downcast_ref::<DbtMetric>() {
            self.__common_attr__.checksum == other_metric.__common_attr__.checksum
        } else {
            false
        }
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtMetric does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtMetric {
    fn set_quoting(&mut self, _quoting: ResolvedQuoting) {
        unimplemented!("")
    }
    fn set_static_analysis(&mut self, _static_analysis: Spanned<StaticAnalysisKind>) {
        unimplemented!("")
    }
    fn search_name(&self) -> String {
        self.name()
    }
    fn selector_string(&self) -> String {
        format!("metric:{}.{}", self.package_name(), self.search_name())
    }
    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtSavedQuery {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }
    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }
    fn resource_type(&self) -> NodeType {
        NodeType::SavedQuery
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }
    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_saved_query) = other.as_any().downcast_ref::<DbtSavedQuery>() {
            self.deprecated_config == other_saved_query.deprecated_config
        } else {
            false
        }
    }
    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        unimplemented!("semantic model content comparison")
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtSavedQuery does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtSavedQuery {
    fn set_quoting(&mut self, _quoting: ResolvedQuoting) {
        unimplemented!("")
    }
    fn set_static_analysis(&mut self, _static_analysis: Spanned<StaticAnalysisKind>) {
        unimplemented!("")
    }
    fn search_name(&self) -> String {
        self.name()
    }
    fn selector_string(&self) -> String {
        format!("saved_query:{}.{}", self.package_name(), self.search_name())
    }
    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtFunction {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Function
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_function) = other.as_any().downcast_ref::<DbtFunction>() {
            self.deprecated_config == other_function.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_function) = other.as_any().downcast_ref::<DbtFunction>() {
            self.__common_attr__.checksum == other_function.__common_attr__.checksum
        } else {
            false
        }
    }

    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        // Functions don't support introspection in the same way as models
        // This could be a no-op or we could add introspection support later
    }
}

impl InternalDbtNodeAttributes for DbtFunction {
    fn static_analysis(&self) -> Spanned<StaticAnalysisKind> {
        self.__base_attr__.static_analysis.clone()
    }

    fn set_quoting(&mut self, quoting: ResolvedQuoting) {
        self.__base_attr__.quoting = quoting;
    }

    fn set_static_analysis(&mut self, static_analysis: Spanned<StaticAnalysisKind>) {
        self.__base_attr__.static_analysis = static_analysis;
    }

    fn get_access(&self) -> Option<Access> {
        Some(self.__function_attr__.access.clone())
    }

    fn get_group(&self) -> Option<String> {
        self.__function_attr__.group.clone()
    }

    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }

    fn selector_string(&self) -> String {
        self.__common_attr__.fqn.join(".")
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}

impl InternalDbtNode for DbtMacro {
    fn common(&self) -> &CommonAttributes {
        unimplemented!("macro common attributes access")
    }
    fn base(&self) -> &NodeBaseAttributes {
        unimplemented!("macro base attributes access")
    }
    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        unimplemented!("macro base attributes mutation")
    }
    fn common_mut(&mut self) -> &mut CommonAttributes {
        unimplemented!("macro common attributes mutation")
    }
    fn resource_type(&self) -> NodeType {
        NodeType::Macro
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }
    fn has_same_config(&self, _other: &dyn InternalDbtNode) -> bool {
        unimplemented!("macro config comparison")
    }
    fn has_same_content(&self, _other: &dyn InternalDbtNode) -> bool {
        unimplemented!("macro content comparison")
    }
    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtMacro does not support setting detected_unsafe");
    }
}

#[derive(Debug, Default, Clone)]
pub struct Nodes {
    pub models: BTreeMap<String, Arc<DbtModel>>,
    pub seeds: BTreeMap<String, Arc<DbtSeed>>,
    pub tests: BTreeMap<String, Arc<DbtTest>>,
    pub unit_tests: BTreeMap<String, Arc<DbtUnitTest>>,
    pub sources: BTreeMap<String, Arc<DbtSource>>,
    pub snapshots: BTreeMap<String, Arc<DbtSnapshot>>,
    pub analyses: BTreeMap<String, Arc<DbtAnalysis>>,
    pub exposures: BTreeMap<String, Arc<DbtExposure>>,
    pub semantic_models: BTreeMap<String, Arc<DbtSemanticModel>>,
    pub metrics: BTreeMap<String, Arc<DbtMetric>>,
    pub saved_queries: BTreeMap<String, Arc<DbtSavedQuery>>,
    pub groups: BTreeMap<String, Arc<DbtGroup>>,
    pub functions: BTreeMap<String, Arc<DbtFunction>>,
}

impl Nodes {
    pub fn deep_clone(&self) -> Self {
        let models = self
            .models
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let seeds = self
            .seeds
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let tests = self
            .tests
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let unit_tests = self
            .unit_tests
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let sources = self
            .sources
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let snapshots = self
            .snapshots
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let analyses = self
            .analyses
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let exposures = self
            .exposures
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let semantic_models = self
            .semantic_models
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let metrics = self
            .metrics
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let saved_queries = self
            .saved_queries
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let groups = self
            .groups
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        let functions = self
            .functions
            .iter()
            .map(|(id, node)| (id.clone(), Arc::new((**node).clone())))
            .collect();
        Nodes {
            models,
            seeds,
            tests,
            unit_tests,
            sources,
            snapshots,
            analyses,
            exposures,
            semantic_models,
            metrics,
            saved_queries,
            groups,
            functions,
        }
    }

    /// Return only the keys of materializable nodes (this excludes exposures and semantic resources)
    pub fn materializable_keys(&self) -> impl Iterator<Item = &String> {
        self.models
            .keys()
            .chain(self.seeds.keys())
            .chain(self.tests.keys())
            .chain(self.unit_tests.keys())
            .chain(self.sources.keys())
            .chain(self.snapshots.keys())
            .chain(self.analyses.keys())
            .chain(self.exposures.keys())
            .chain(self.functions.keys())
            .chain(self.semantic_models.keys())
            .chain(self.metrics.keys())
            .chain(self.saved_queries.keys())
    }

    pub fn get_node(&self, unique_id: &str) -> Option<&dyn InternalDbtNodeAttributes> {
        self.models
            .get(unique_id)
            .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            .or_else(|| {
                self.seeds
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.tests
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.unit_tests
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.sources
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.snapshots
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.analyses
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.exposures
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.functions
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.semantic_models
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.metrics
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.saved_queries
                    .get(unique_id)
                    .map(|n| Arc::as_ref(n) as &dyn InternalDbtNodeAttributes)
            })
    }

    pub fn get_node_owned(&self, unique_id: &str) -> Option<Arc<dyn InternalDbtNodeAttributes>> {
        self.models
            .get(unique_id)
            .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            .or_else(|| {
                self.seeds
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.tests
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.unit_tests
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.sources
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.snapshots
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.analyses
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.exposures
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.functions
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.semantic_models
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.metrics
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
            .or_else(|| {
                self.saved_queries
                    .get(unique_id)
                    .map(|n| n.clone() as Arc<dyn InternalDbtNodeAttributes>)
            })
    }

    /// Check if a node exists in the graph.
    /// Used with [`Nodes::materializable_keys`], so intent could be to only check for materializable nodes.
    /// TODO: Determine if this function should be updated to only check for materializable nodes.
    pub fn contains(&self, unique_id: &str) -> bool {
        self.models.contains_key(unique_id)
            || self.seeds.contains_key(unique_id)
            || self.tests.contains_key(unique_id)
            || self.unit_tests.contains_key(unique_id)
            || self.sources.contains_key(unique_id)
            || self.snapshots.contains_key(unique_id)
            || self.analyses.contains_key(unique_id)
            || self.exposures.contains_key(unique_id)
            || self.semantic_models.contains_key(unique_id)
            || self.metrics.contains_key(unique_id)
            || self.functions.contains_key(unique_id)
            || self.saved_queries.contains_key(unique_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &dyn InternalDbtNodeAttributes)> + '_ {
        self.models
            .iter()
            .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes))
            .chain(
                self.seeds
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.tests
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.unit_tests
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.sources
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.snapshots
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.analyses
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.exposures
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.functions
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.semantic_models
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.metrics
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
            .chain(
                self.saved_queries
                    .iter()
                    .map(|(id, node)| (id, Arc::as_ref(node) as &dyn InternalDbtNodeAttributes)),
            )
    }

    pub fn into_iter(
        &self,
    ) -> impl Iterator<Item = (String, Arc<dyn InternalDbtNodeAttributes>)> + '_ {
        let models = self
            .models
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let seeds = self
            .seeds
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let tests = self
            .tests
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let unit_tests = self
            .unit_tests
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let sources = self
            .sources
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let snapshots = self
            .snapshots
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let analyses = self
            .analyses
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let exposures = self
            .exposures
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let functions = self
            .functions
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let semantic_models = self
            .semantic_models
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let metrics = self
            .metrics
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));
        let saved_queries = self
            .saved_queries
            .iter()
            .map(|(id, node)| (id.clone(), upcast(node.clone())));

        models
            .chain(seeds)
            .chain(tests)
            .chain(unit_tests)
            .chain(sources)
            .chain(snapshots)
            .chain(analyses)
            .chain(exposures)
            .chain(functions)
            .chain(semantic_models)
            .chain(metrics)
            .chain(saved_queries)
    }

    pub fn iter_values_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut dyn InternalDbtNodeAttributes> + '_ {
        let map_models = self
            .models
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_seeds = self
            .seeds
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_tests = self
            .tests
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_unit_tests = self
            .unit_tests
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_sources = self
            .sources
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_snapshots = self
            .snapshots
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_analyses = self
            .analyses
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_exposures = self
            .exposures
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_functions = self
            .functions
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_semantic_models = self
            .semantic_models
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_metrics = self
            .metrics
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);
        let map_saved_queries = self
            .saved_queries
            .values_mut()
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes);

        map_models
            .chain(map_seeds)
            .chain(map_tests)
            .chain(map_unit_tests)
            .chain(map_sources)
            .chain(map_snapshots)
            .chain(map_analyses)
            .chain(map_exposures)
            .chain(map_functions)
            .chain(map_semantic_models)
            .chain(map_metrics)
            .chain(map_saved_queries)
    }

    pub fn get_value_mut(&mut self, unique_id: &str) -> Option<&mut dyn InternalDbtNodeAttributes> {
        self.models
            .get_mut(unique_id)
            .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            .or_else(|| {
                self.seeds
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.tests
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.unit_tests
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.sources
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.snapshots
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.analyses
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.exposures
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.functions
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.semantic_models
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.metrics
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.saved_queries
                    .get_mut(unique_id)
                    .map(|arc| Arc::make_mut(arc) as &mut dyn InternalDbtNodeAttributes)
            })
    }

    pub fn get_by_relation_name(
        &self,
        relation_name: &str,
    ) -> Option<&dyn InternalDbtNodeAttributes> {
        self.models
            .values()
            .find(|model| model.base().relation_name == Some(relation_name.to_string()))
            .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            .or_else(|| {
                self.seeds
                    .values()
                    .find(|seed| seed.base().relation_name == Some(relation_name.to_string()))
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.tests
                    .values()
                    .find(|test| test.base().relation_name == Some(relation_name.to_string()))
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.unit_tests
                    .values()
                    .find(|unit_test| {
                        unit_test.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.sources
                    .values()
                    .find(|source| source.base().relation_name == Some(relation_name.to_string()))
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.snapshots
                    .values()
                    .find(|snapshot| {
                        snapshot.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.analyses
                    .values()
                    .find(|analysis| {
                        analysis.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.exposures
                    .values()
                    .find(|exposure| {
                        exposure.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.functions
                    .values()
                    .find(|function| {
                        function.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.semantic_models
                    .values()
                    .find(|semantic_model| {
                        semantic_model.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.metrics
                    .values()
                    .find(|metric| metric.base().relation_name == Some(relation_name.to_string()))
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
            .or_else(|| {
                self.saved_queries
                    .values()
                    .find(|saved_query| {
                        saved_query.base().relation_name == Some(relation_name.to_string())
                    })
                    .map(|arc| Arc::as_ref(arc) as &dyn InternalDbtNodeAttributes)
            })
    }

    pub fn extend(&mut self, other: Nodes) {
        self.models.extend(other.models);
        self.seeds.extend(other.seeds);
        self.tests.extend(other.tests);
        self.unit_tests.extend(other.unit_tests);
        self.sources.extend(other.sources);
        self.snapshots.extend(other.snapshots);
        self.analyses.extend(other.analyses);
        self.exposures.extend(other.exposures);
        self.semantic_models.extend(other.semantic_models);
        self.metrics.extend(other.metrics);
        self.saved_queries.extend(other.saved_queries);
        self.groups.extend(other.groups);
        self.functions.extend(other.functions);
    }

    pub fn warn_on_custom_materializations(&self) -> FsResult<()> {
        let mut custom_materializations: Vec<(String, String)> = Vec::new();

        for (_, node) in self.iter() {
            if let DbtMaterialization::Unknown(custom) = node.materialized() {
                custom_materializations.push((node.common().unique_id.clone(), custom));
            }
        }

        if !custom_materializations.is_empty() {
            let mut message = "Custom materialization macros are not supported. Found custom materializations in the following nodes:\n".to_string();
            for (unique_id, materialization) in &custom_materializations {
                message.push_str(&format!(
                    "  - {unique_id} (materialization: {materialization})\n"
                ));
            }

            return err!(ErrorCode::UnsupportedFeature, "{}", message);
        }
        Ok(())
    }

    pub fn warn_on_microbatch(&self) -> FsResult<()> {
        for (_, node) in self.iter() {
            node.warn_on_microbatch()?;
        }
        Ok(())
    }
}

fn upcast<T: InternalDbtNodeAttributes + 'static>(
    arc: Arc<T>,
) -> Arc<dyn InternalDbtNodeAttributes> {
    arc
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct CommonAttributes {
    // Identifiers
    pub unique_id: String,
    pub name: String,
    pub package_name: String,
    pub fqn: Vec<String>,

    // Paths
    pub path: PathBuf,

    /// The original file path where this node was defined
    ///
    /// **NOTE**: For [DbtTest] nodes, this is currently the path to the
    /// generated SQL file, *NOT* the path to the YAML file where the test was
    /// defined!
    pub original_file_path: PathBuf,

    pub raw_code: Option<String>,
    pub patch_path: Option<PathBuf>,
    pub name_span: dbt_common::Span,

    // Checksum
    pub checksum: DbtChecksum,
    pub language: Option<String>,

    // Meta
    pub description: Option<String>,

    // Tags and Meta
    pub tags: Vec<String>,
    pub meta: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct NodeBaseAttributes {
    // Identifiers
    #[serde(default)]
    pub database: String,
    pub schema: String,
    pub alias: String,
    pub relation_name: Option<String>,

    // Resolved Quoting
    pub quoting: ResolvedQuoting,
    // TODO: Potentially add ignore_case to ResolvedQuoting
    pub quoting_ignore_case: bool,
    pub materialized: DbtMaterialization,
    pub static_analysis: Spanned<StaticAnalysisKind>,
    #[serde(skip_serializing, default)]
    pub static_analysis_off_reason: Option<StaticAnalysisOffReason>,
    pub enabled: bool,
    #[serde(skip_serializing, default = "default_false")]
    pub extended_model: bool,

    // Documentation persistence configuration
    pub persist_docs: Option<PersistDocsConfig>,

    // Derived
    #[serde(
        default,
        serialize_with = "serialize_dbt_columns",
        deserialize_with = "deserialize_dbt_columns"
    )]
    pub columns: Vec<DbtColumnRef>,

    // Raw Refs, Source, and Metric Dependencies from SQL
    #[serde(default)]
    pub refs: Vec<DbtRef>,
    #[serde(default)]
    pub sources: Vec<DbtSourceWrapper>,
    #[serde(default)]
    pub functions: Vec<DbtRef>,
    #[serde(default)]
    pub metrics: Vec<Vec<String>>,

    // Resolved Dependencies
    pub depends_on: NodeDependsOn,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSeed {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __seed_attr__: DbtSeedAttr,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: SeedConfig,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSeedAttr {
    #[serde(default, skip_serializing_if = "is_false")]
    pub quote_columns: bool,
    pub column_types: Option<BTreeMap<Spanned<String>, String>>,
    pub delimiter: Option<String>,
    pub root_path: Option<PathBuf>,
}

fn is_false(b: &bool) -> bool {
    !b
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DbtExposure {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __exposure_attr__: DbtExposureAttr,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: ExposureConfig,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtExposureAttr {
    pub owner: DbtOwner,
    pub label: Option<String>,
    pub maturity: Option<String>,
    #[serde(rename = "type")]
    pub type_: String,
    pub url: Option<String>,
    pub unrendered_config: BTreeMap<String, YmlValue>,
    pub created_at: Option<f64>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtGroup {
    pub __common_attr__: CommonAttributes,
    pub __base_attr__: NodeBaseAttributes,
    pub __group_attr__: DbtGroupAttr,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtGroupAttr {
    pub owner: DbtOwner,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtUnitTest {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __unit_test_attr__: DbtUnitTestAttr,

    #[serde(rename = "_event_status")]
    pub field_event_status: Option<BTreeMap<String, YmlValue>>,
    #[serde(rename = "_pre_injected_sql")]
    pub field_pre_injected_sql: Option<String>,
    pub tested_node_unique_id: Option<String>,
    pub this_input_node_unique_id: Option<String>,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: UnitTestConfig,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub struct DbtUnitTestAttr {
    pub model: String,
    pub given: Vec<Given>,
    pub expect: Expect,
    pub versions: Option<IncludeExclude>,
    pub version: Option<StringOrInteger>,
    pub overrides: Option<UnitTestOverrides>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtTest {
    pub __common_attr__: CommonAttributes,
    pub __base_attr__: NodeBaseAttributes,
    pub __test_attr__: DbtTestAttr,
    pub defined_at: Option<dbt_common::CodeLocation>,

    // not to be confused with __common_attr__.original_file_path, which is the path to the generated sql file
    pub manifest_original_file_path: PathBuf,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: DataTestConfig,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtTestAttr {
    pub column_name: Option<String>,
    pub attached_node: Option<String>,
    pub test_metadata: Option<TestMetadata>,
    pub file_key_name: Option<String>,
    #[serde(skip_serializing, default = "default_introspection")]
    pub introspection: IntrospectionKind,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct TestMetadata {
    pub name: String,
    pub kwargs: BTreeMap<String, YmlValue>,
    pub namespace: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSnapshot {
    pub __common_attr__: CommonAttributes,
    pub __base_attr__: NodeBaseAttributes,
    pub __snapshot_attr__: DbtSnapshotAttr,
    pub __adapter_attr__: AdapterAttr,

    /// To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: SnapshotConfig,
    // TODO: Deprecate compiled and compiled_code fields (This field is used by the materialization
    // macro when this node is passed into the jinja context)
    pub compiled: Option<bool>,
    pub compiled_code: Option<String>,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSnapshotAttr {
    pub snapshot_meta_column_names: SnapshotMetaColumnNames,
    #[serde(skip_serializing, default = "default_introspection")]
    pub introspection: IntrospectionKind,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSource {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    // Source Specific Attributes
    pub __source_attr__: DbtSourceAttr,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: SourceConfig,

    // Other untyped (or rather externally typed) keys that can be used by dbt packages.
    // For example, the `external` key is used by `dbt-external-tables`, but it's not
    // explicitly typed by dbt itself.
    // See: https://github.com/dbt-labs/dbt-external-tables
    pub __other__: BTreeMap<String, YmlValue>,
}
impl DbtSource {
    pub fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }
    pub fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtSourceAttr {
    pub identifier: String,
    pub source_name: String,
    pub source_description: String,
    pub loader: String,
    pub loaded_at_field: Option<String>,
    pub loaded_at_query: Option<String>,
    #[serialize_always]
    pub freshness: Option<FreshnessDefinition>,
}

impl DbtSource {
    pub fn get_base_attr(&self) -> NodeBaseAttributes {
        self.__base_attr__.clone()
    }

    pub fn get_loaded_at_field(&self) -> &str {
        self.__source_attr__
            .loaded_at_field
            .as_ref()
            .map(AsRef::as_ref)
            .unwrap_or("")
    }

    pub fn get_loaded_at_query(&self) -> &str {
        self.__source_attr__
            .loaded_at_query
            .as_ref()
            .map(AsRef::as_ref)
            .unwrap_or("")
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtModel {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __model_attr__: DbtModelAttr,

    pub __adapter_attr__: AdapterAttr,

    // TO BE DEPRECATED
    #[serde(rename = "config")]
    pub deprecated_config: ModelConfig,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct AdapterAttr {
    pub snowflake_attr: Option<Box<SnowflakeAttr>>,
    pub databricks_attr: Option<Box<DatabricksAttr>>,
    pub bigquery_attr: Option<Box<BigQueryAttr>>,
    pub redshift_attr: Option<Box<RedshiftAttr>>,
}

impl AdapterAttr {
    pub fn with_snowflake_attr(mut self, snowflake_attr: Option<Box<SnowflakeAttr>>) -> Self {
        self.snowflake_attr = snowflake_attr;
        self
    }

    pub fn with_databricks_attr(mut self, databricks_attr: Option<Box<DatabricksAttr>>) -> Self {
        self.databricks_attr = databricks_attr;
        self
    }

    pub fn with_bigquery_attr(mut self, bigquery_attr: Option<Box<BigQueryAttr>>) -> Self {
        self.bigquery_attr = bigquery_attr;
        self
    }

    pub fn with_redshift_attr(mut self, redshift_attr: Option<Box<RedshiftAttr>>) -> Self {
        self.redshift_attr = redshift_attr;
        self
    }

    /// Creates a new [AdapterAttr] from a given [&WarehouseSpecificNodeConfig] and adapter type string
    pub fn from_config_and_dialect(
        config: &WarehouseSpecificNodeConfig,
        adapter_type: AdapterType,
    ) -> Self {
        // TODO: Make all None inputs result in a None for that attr
        // TODO: Creation of configs should be delegated to the adapter specific attr struct
        // Here we naively initialize the resolved adapter attributes.
        // This allows propagation of configurations and switching on the adapter type
        // allows us to save memory for unused configurations.
        //
        // A further optimization can be done by only setting inner fields of [AdapterAttr] if at least one field of the
        // input is not None
        match adapter_type {
            AdapterType::Snowflake => {
                AdapterAttr::default().with_snowflake_attr(Some(Box::new(SnowflakeAttr {
                    table_tag: config.table_tag.clone(),
                    row_access_policy: config.row_access_policy.clone(),
                    adapter_properties: config.adapter_properties.clone(),
                    external_volume: config.external_volume.clone(),
                    base_location_root: config.base_location_root.clone(),
                    base_location_subpath: config.base_location_subpath.clone(),
                    target_lag: config.target_lag.clone(),
                    snowflake_warehouse: config.snowflake_warehouse.clone(),
                    refresh_mode: config.refresh_mode.clone(),
                    initialize: config.initialize.clone(),
                    tmp_relation_type: config.tmp_relation_type.clone(),
                    query_tag: config.query_tag.clone(),
                    automatic_clustering: config.automatic_clustering,
                    copy_grants: config.copy_grants,
                    secure: config.secure,
                    transient: config.transient,
                })))
            }
            AdapterType::Postgres => AdapterAttr::default(),
            AdapterType::Bigquery => {
                AdapterAttr::default().with_bigquery_attr(Some(Box::new(BigQueryAttr {
                    adapter_properties: config.adapter_properties.clone(),
                    external_volume: config.external_volume.clone(),
                    base_location_root: config.base_location_root.clone(),
                    base_location_subpath: config.base_location_subpath.clone(),
                    file_format: config.file_format.clone(),
                    partition_by: config.partition_by.clone(),
                    cluster_by: config.cluster_by.clone(),
                    hours_to_expiration: config.hours_to_expiration,
                    labels: config.labels.clone(),
                    labels_from_meta: config.labels_from_meta,
                    kms_key_name: config.kms_key_name.clone(),
                    require_partition_filter: config.require_partition_filter,
                    partition_expiration_days: config.partition_expiration_days,
                    grant_access_to: config.grant_access_to.clone(),
                    partitions: config.partitions.clone(),
                    enable_refresh: config.enable_refresh,
                    refresh_interval_minutes: config.refresh_interval_minutes,
                    max_staleness: config.max_staleness.clone(),
                })))
            }
            AdapterType::Redshift => {
                AdapterAttr::default().with_redshift_attr(Some(Box::new(RedshiftAttr {
                    auto_refresh: config.auto_refresh,
                    backup: config.backup,
                    bind: config.bind,
                    dist: config.dist.clone(),
                    sort: config.sort.clone(),
                    sort_type: config.sort_type.clone(),
                })))
            }
            AdapterType::Databricks => {
                AdapterAttr::default().with_databricks_attr(Some(Box::new(DatabricksAttr {
                    adapter_properties: config.adapter_properties.clone(),
                    partition_by: config.partition_by.clone(),
                    file_format: config.file_format.clone(),
                    location_root: config.location_root.clone(),
                    tblproperties: config.tblproperties.clone(),
                    include_full_name_in_path: config.include_full_name_in_path,
                    liquid_clustered_by: config.liquid_clustered_by.clone(),
                    auto_liquid_cluster: config.auto_liquid_cluster,
                    clustered_by: config.clustered_by.clone(),
                    buckets: config.buckets,
                    catalog: config.catalog.clone(),
                    databricks_tags: config.databricks_tags.clone(),
                    compression: config.compression.clone(),
                    databricks_compute: config.databricks_compute.clone(),
                    target_alias: config.target_alias.clone(),
                    source_alias: config.source_alias.clone(),
                    matched_condition: config.matched_condition.clone(),
                    not_matched_condition: config.not_matched_condition.clone(),
                    not_matched_by_source_condition: config.not_matched_by_source_condition.clone(),
                    not_matched_by_source_action: config.not_matched_by_source_action.clone(),
                    merge_with_schema_evolution: config.merge_with_schema_evolution,
                    skip_matched_step: config.skip_matched_step,
                    skip_not_matched_step: config.skip_not_matched_step,
                    schedule: config.schedule.clone(),
                })))
            }
            _ => {
                // Unknown input, populate ALL adapter attributes to maximize compatibility downstream
                AdapterAttr::default()
                    .with_snowflake_attr(Some(Box::new(SnowflakeAttr {
                        table_tag: config.table_tag.clone(),
                        row_access_policy: config.row_access_policy.clone(),
                        adapter_properties: config.adapter_properties.clone(),
                        external_volume: config.external_volume.clone(),
                        base_location_root: config.base_location_root.clone(),
                        base_location_subpath: config.base_location_subpath.clone(),
                        target_lag: config.target_lag.clone(),
                        snowflake_warehouse: config.snowflake_warehouse.clone(),
                        refresh_mode: config.refresh_mode.clone(),
                        initialize: config.initialize.clone(),
                        tmp_relation_type: config.tmp_relation_type.clone(),
                        query_tag: config.query_tag.clone(),
                        automatic_clustering: config.automatic_clustering,
                        copy_grants: config.copy_grants,
                        secure: config.secure,
                        transient: config.transient,
                    })))
                    .with_bigquery_attr(Some(Box::new(BigQueryAttr {
                        adapter_properties: config.adapter_properties.clone(),
                        external_volume: config.external_volume.clone(),
                        base_location_root: config.base_location_root.clone(),
                        base_location_subpath: config.base_location_subpath.clone(),
                        file_format: config.file_format.clone(),
                        partition_by: config.partition_by.clone(),
                        cluster_by: config.cluster_by.clone(),
                        hours_to_expiration: config.hours_to_expiration,
                        labels: config.labels.clone(),
                        labels_from_meta: config.labels_from_meta,
                        kms_key_name: config.kms_key_name.clone(),
                        require_partition_filter: config.require_partition_filter,
                        partition_expiration_days: config.partition_expiration_days,
                        grant_access_to: config.grant_access_to.clone(),
                        partitions: config.partitions.clone(),
                        enable_refresh: config.enable_refresh,
                        refresh_interval_minutes: config.refresh_interval_minutes,
                        max_staleness: config.max_staleness.clone(),
                    })))
                    .with_redshift_attr(Some(Box::new(RedshiftAttr {
                        auto_refresh: config.auto_refresh,
                        backup: config.backup,
                        bind: config.bind,
                        dist: config.dist.clone(),
                        sort: config.sort.clone(),
                        sort_type: config.sort_type.clone(),
                    })))
                    .with_databricks_attr(Some(Box::new(DatabricksAttr {
                        adapter_properties: config.adapter_properties.clone(),
                        partition_by: config.partition_by.clone(),
                        file_format: config.file_format.clone(),
                        location_root: config.location_root.clone(),
                        tblproperties: config.tblproperties.clone(),
                        include_full_name_in_path: config.include_full_name_in_path,
                        liquid_clustered_by: config.liquid_clustered_by.clone(),
                        auto_liquid_cluster: config.auto_liquid_cluster,
                        clustered_by: config.clustered_by.clone(),
                        buckets: config.buckets,
                        catalog: config.catalog.clone(),
                        databricks_tags: config.databricks_tags.clone(),
                        compression: config.compression.clone(),
                        databricks_compute: config.databricks_compute.clone(),
                        target_alias: config.target_alias.clone(),
                        source_alias: config.source_alias.clone(),
                        matched_condition: config.matched_condition.clone(),
                        not_matched_condition: config.not_matched_condition.clone(),
                        not_matched_by_source_condition: config
                            .not_matched_by_source_condition
                            .clone(),
                        not_matched_by_source_action: config.not_matched_by_source_action.clone(),
                        merge_with_schema_evolution: config.merge_with_schema_evolution,
                        skip_matched_step: config.skip_matched_step,
                        skip_not_matched_step: config.skip_not_matched_step,
                        schedule: config.schedule.clone(),
                    })))
            }
        }
    }
}

/// A resolved Snowflake configuration
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SnowflakeAttr {
    pub adapter_properties: Option<BTreeMap<String, YmlValue>>,
    pub table_tag: Option<String>,
    pub row_access_policy: Option<String>,
    pub external_volume: Option<String>,
    pub base_location_root: Option<String>,
    pub base_location_subpath: Option<String>,
    pub target_lag: Option<String>,
    pub snowflake_warehouse: Option<String>,
    pub refresh_mode: Option<String>,
    pub initialize: Option<String>,
    pub tmp_relation_type: Option<String>,
    pub query_tag: Option<String>,
    pub automatic_clustering: Option<bool>,
    pub copy_grants: Option<bool>,
    pub secure: Option<bool>,
    pub transient: Option<bool>,
}

/// A resolved Databricks configuration
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabricksAttr {
    pub adapter_properties: Option<BTreeMap<String, YmlValue>>,
    pub partition_by: Option<PartitionConfig>,
    pub file_format: Option<String>,
    pub location_root: Option<String>,
    pub tblproperties: Option<BTreeMap<String, YmlValue>>,
    pub include_full_name_in_path: Option<bool>,
    pub liquid_clustered_by: Option<StringOrArrayOfStrings>,
    pub auto_liquid_cluster: Option<bool>,
    pub clustered_by: Option<String>,
    pub buckets: Option<i64>,
    pub catalog: Option<String>,
    pub databricks_tags: Option<BTreeMap<String, YmlValue>>,
    pub compression: Option<String>,
    pub databricks_compute: Option<String>,
    pub target_alias: Option<String>,
    pub source_alias: Option<String>,
    pub matched_condition: Option<String>,
    pub not_matched_condition: Option<String>,
    pub not_matched_by_source_condition: Option<String>,
    pub not_matched_by_source_action: Option<String>,
    pub merge_with_schema_evolution: Option<bool>,
    pub skip_matched_step: Option<bool>,
    pub skip_not_matched_step: Option<bool>,
    pub schedule: Option<ScheduleConfig>,
}

/// A resolved BigQuery configuration
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BigQueryAttr {
    pub adapter_properties: Option<BTreeMap<String, YmlValue>>,
    pub external_volume: Option<String>,
    pub file_format: Option<String>,
    pub base_location_root: Option<String>,
    pub base_location_subpath: Option<String>,
    pub partition_by: Option<PartitionConfig>,
    pub cluster_by: Option<BigqueryClusterConfig>,
    pub hours_to_expiration: Option<u64>,
    pub labels: Option<BTreeMap<String, String>>,
    pub labels_from_meta: Option<bool>,
    pub kms_key_name: Option<String>,
    pub require_partition_filter: Option<bool>,
    pub partition_expiration_days: Option<u64>,
    pub grant_access_to: Option<Vec<GrantAccessToTarget>>,
    pub partitions: Option<Vec<String>>,
    pub enable_refresh: Option<bool>,
    pub refresh_interval_minutes: Option<f64>,
    pub max_staleness: Option<String>,
}

/// A resolved Redshift configuration
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RedshiftAttr {
    pub auto_refresh: Option<bool>,
    pub backup: Option<bool>,
    pub bind: Option<bool>,
    pub dist: Option<String>,
    pub sort: Option<StringOrArrayOfStrings>,
    pub sort_type: Option<String>,
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DbtModelAttr {
    pub access: Access,
    pub group: Option<String>,
    #[serde(skip_serializing, default = "default_introspection")]
    pub introspection: IntrospectionKind,
    pub contract: Option<DbtContract>,
    pub incremental_strategy: Option<DbtIncrementalStrategy>,
    pub freshness: Option<ModelFreshness>,
    pub version: Option<StringOrInteger>,
    pub latest_version: Option<StringOrInteger>,
    pub constraints: Vec<ModelConstraint>,
    pub deprecation_date: Option<String>,
    // TODO: Investigate why primary_key is needed here (constraints already exist)
    pub primary_key: Vec<String>,
    pub time_spine: Option<TimeSpine>,
    pub event_time: Option<String>,
    // TODO(anna): See if we _need_ to put these here, or if they can somehow be added to AdapterAttr.
    pub catalog_name: Option<String>,
    pub table_format: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtFunction {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __function_attr__: DbtFunctionAttr,

    // To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: FunctionConfig,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtFunctionAttr {
    pub access: Access,
    pub group: Option<String>,
    pub language: Option<String>,
    pub on_configuration_change: Option<String>,
    pub returns: Option<FunctionReturnType>,
    pub arguments: Option<Vec<FunctionArgument>>,
    #[serde(rename = "type")]
    pub function_kind: FunctionKind,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TimeSpine {
    pub node_relation: NodeRelation,
    pub primary_column: TimeSpinePrimaryColumn,
    pub custom_granularities:
        Vec<crate::schemas::properties::model_properties::TimeSpineCustomGranularity>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TimeSpinePrimaryColumn {
    pub name: String,
    pub time_granularity: crate::schemas::dbt_column::Granularity,
}

fn default_introspection() -> IntrospectionKind {
    IntrospectionKind::None
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::ModelConfig;

    type YmlValue = dbt_serde_yaml::Value;

    #[test]
    fn test_deserialize_wo_meta() {
        let config: YmlValue = dbt_serde_yaml::from_str(
            r#"
            enabled: true
            "#,
        )
        .expect("Failed to deserialize model config");

        let config = ModelConfig::deserialize(config);
        if let Err(err) = config {
            panic!("Could not deserialize and failed with the following error: {err}");
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DbtAnalysis {
    pub __common_attr__: CommonAttributes,

    pub __base_attr__: NodeBaseAttributes,

    pub __analysis_attr__: DbtAnalysisAttr,

    /// To be deprecated
    #[serde(rename = "config")]
    pub deprecated_config: super::project::configs::analysis_config::AnalysesConfig,

    pub __other__: BTreeMap<String, YmlValue>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct DbtAnalysisAttr {
    // Analysis nodes have minimal attributes since they're for ad-hoc querying
    // Most functionality comes from the base attributes and config
}

impl InternalDbtNode for DbtAnalysis {
    fn common(&self) -> &CommonAttributes {
        &self.__common_attr__
    }

    fn base(&self) -> &NodeBaseAttributes {
        &self.__base_attr__
    }

    fn base_mut(&mut self) -> &mut NodeBaseAttributes {
        &mut self.__base_attr__
    }

    fn common_mut(&mut self) -> &mut CommonAttributes {
        &mut self.__common_attr__
    }

    fn resource_type(&self) -> NodeType {
        NodeType::Analysis
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn serialize_inner(&self) -> YmlValue {
        dbt_serde_yaml::to_value(self).expect("Failed to serialize to YAML")
    }

    fn has_same_config(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_analysis) = other.as_any().downcast_ref::<DbtAnalysis>() {
            self.deprecated_config == other_analysis.deprecated_config
        } else {
            false
        }
    }

    fn has_same_content(&self, other: &dyn InternalDbtNode) -> bool {
        if let Some(other_analysis) = other.as_any().downcast_ref::<DbtAnalysis>() {
            self.__common_attr__.checksum == other_analysis.__common_attr__.checksum
        } else {
            false
        }
    }

    fn set_detected_introspection(&mut self, _introspection: IntrospectionKind) {
        panic!("DbtAnalysis does not support setting detected_unsafe");
    }
}

impl InternalDbtNodeAttributes for DbtAnalysis {
    fn static_analysis(&self) -> Spanned<StaticAnalysisKind> {
        self.__base_attr__.static_analysis.clone()
    }

    fn set_quoting(&mut self, quoting: ResolvedQuoting) {
        self.__base_attr__.quoting = quoting;
    }

    fn set_static_analysis(&mut self, static_analysis: Spanned<StaticAnalysisKind>) {
        self.__base_attr__.static_analysis = static_analysis;
    }

    fn get_access(&self) -> Option<Access> {
        None // Analysis nodes don't have access controls
    }

    fn get_group(&self) -> Option<String> {
        None // Analysis nodes don't have groups
    }

    fn search_name(&self) -> String {
        self.__common_attr__.name.clone()
    }

    fn selector_string(&self) -> String {
        format!("analysis:{}", self.__common_attr__.name)
    }

    fn serialized_config(&self) -> YmlValue {
        dbt_serde_yaml::to_value(&self.deprecated_config).expect("Failed to serialize to YAML")
    }
}
