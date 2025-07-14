pub mod constants;
pub mod dbt_types;
pub mod dbt_utils;
pub mod man;
pub mod state;

pub mod schemas {
    pub mod columns;
    pub mod common;
    pub mod data_tests;
    pub mod dbt_column;
    pub mod macros;
    pub mod packages;
    mod prev_state;
    pub mod profiles;
    pub mod ref_and_source;
    pub mod relations;
    mod run_results;
    pub mod selectors;
    pub mod serde;
    mod sources;
    pub use prev_state::{ModificationType, PreviousState};
    pub use run_results::{
        RunResult, RunResultsArgs, RunResultsArtifact, RunResultsMetadata, TimingInfo,
    };

    // Add re-exports from relation_configs
    pub use relations::relation_configs::{
        BaseRelationChangeSet, BaseRelationConfig, ComponentConfig, RelationChangeSet,
        RelationConfigFactory,
    };

    mod nodes;
    pub use nodes::{
        CommonAttributes, DbtModel, DbtModelAttr, DbtSeed, DbtSeedAttr, DbtSnapshot,
        DbtSnapshotAttr, DbtSource, DbtSourceAttr, DbtTest, DbtTestAttr, DbtUnitTest,
        DbtUnitTestAttr, InternalDbtNode, InternalDbtNodeAttributes, InternalDbtNodeWrapper,
        IntrospectionKind, NodeBaseAttributes, Nodes, TestMetadata,
    };

    pub use sources::{FreshnessResultsArtifact, FreshnessResultsMetadata, FreshnessResultsNode};
    pub mod manifest {
        mod bigquery_partition;
        mod exposure;
        mod group;
        #[allow(clippy::module_inception)]
        mod manifest;
        mod manifest_nodes;
        mod metric;
        mod operation;
        mod saved_query;
        mod selector;
        mod semantic_model;

        pub mod common;
        pub use bigquery_partition::{
            BigqueryClusterConfig, BigqueryPartitionConfig, BigqueryPartitionConfigInner,
            BigqueryPartitionConfigLegacy, GrantAccessToTarget, Range, RangeConfig, TimeConfig,
        };
        pub use exposure::DbtExposure;
        pub use group::DbtGroup;
        pub use manifest::{
            build_manifest, nodes_from_dbt_manifest, BaseMetadata, DbtManifest, DbtNode,
            ManifestMetadata,
        };
        pub use manifest_nodes::{
            ManifestDataTest, ManifestModel, ManifestSeed, ManifestSnapshot, ManifestSource,
            ManifestUnitTest,
        };
        pub use metric::DbtMetric;
        pub use operation::DbtOperation;
        pub use saved_query::DbtSavedQuery;
        pub use selector::DbtSelector;
        pub use semantic_model::DbtSemanticModel;
    }
    mod dbt_cloud;
    pub use dbt_cloud::{DbtCloudConfig, DbtCloudContext, DbtCloudProject};
    pub mod project {
        mod dbt_project;
        mod configs {
            pub mod common;
            pub mod data_test_config;
            pub mod exposure_config;
            pub mod metric_config;
            pub mod model_config;
            pub mod omissible_utils;
            pub mod omissible_utils_tests;
            pub mod saved_queries_config;
            pub mod seed_config;
            pub mod semantic_model_config;
            pub mod snapshot_config;
            pub mod source_config;
            pub mod unit_test_config;
        }

        pub use configs::common::{BigQueryNodeConfig, DatabricksNodeConfig, SnowflakeNodeConfig};
        pub use configs::data_test_config::{DataTestConfig, ProjectDataTestConfig};
        pub use configs::exposure_config::{ExposureConfig, ProjectExposureConfig};
        pub use configs::metric_config::{MetricConfig, ProjectMetricConfigs};
        pub use configs::model_config::{ModelConfig, ProjectModelConfig};
        pub use configs::saved_queries_config::{
            ExportConfigExportAs, SavedQueriesConfig, SavedQueriesConfigCache,
        };
        pub use configs::seed_config::{ProjectSeedConfig, SeedConfig};
        pub use configs::semantic_model_config::{ProjectSemanticModelConfig, SemanticModelConfig};
        pub use configs::snapshot_config::{
            ProjectSnapshotConfig, SnapshotConfig, SnapshotMetaColumnNames,
        };
        pub use configs::source_config::{ProjectSourceConfig, SourceConfig};
        pub use configs::unit_test_config::{ProjectUnitTestConfig, UnitTestConfig};
        pub use dbt_project::{
            DbtProject, DbtProjectSimplified, DefaultTo, IterChildren, ProjectDbtCloudConfig,
            QueryComment,
        };
    }

    pub mod properties {
        mod data_test_properties;
        mod exposure_properties;
        mod metrics_properties;
        mod model_properties;
        #[allow(clippy::module_inception)]
        mod properties;
        mod saved_queries_properties;
        mod seed_properties;
        mod semantic_models_properties;
        mod snapshot_properties;
        mod source_properties;
        mod unit_test_properties;

        pub use data_test_properties::DataTestProperties;
        pub use exposure_properties::ExposureProperties;
        pub use metrics_properties::MetricsProperties;
        pub use model_properties::ModelConstraint;
        pub use model_properties::ModelFreshness;
        pub use model_properties::ModelProperties;
        pub use properties::{
            DbtPropertiesFile, DbtPropertiesFileValues, GetConfig, MinimalSchemaValue,
            MinimalTableValue,
        };
        pub use saved_queries_properties::SavedQueriesProperties;
        pub use seed_properties::SeedProperties;
        pub use semantic_models_properties::SemanticModelsProperties;
        pub use snapshot_properties::SnapshotProperties;
        pub use source_properties::{SourceProperties, Tables};
        pub use unit_test_properties::{UnitTestOverrides, UnitTestProperties};
    }
}
