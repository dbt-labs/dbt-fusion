use std::fmt::Display;

use int_enum::IntEnum;

/// Error codes for the SDF Frontend.
///
/// Each error code is a 3-digit number in the range [100, 999], stored as a u16
/// type.
#[repr(u16)]
#[non_exhaustive]
#[derive(Debug, Copy, Clone, Eq, PartialEq, IntEnum)]
pub enum ErrorCode {
    SyntaxError = 101,
    SchemaError = 102,
    MalformedExpression = 103,
    DanglingComma = 104,
    AmbiguousColumnReference = 105,
    FunctionCaseMismatch = 106,
    KeywordCaseMismatch = 107,
    BooleanLiteralCaseMismatch = 108,
    NullLiteralCaseMismatch = 109,
    PrimitiveTypeCaseMismatch = 110,
    StatementMustEndWithSemicolon = 111,
    InconsistentReference = 112,
    PreferCTE = 113,
    JoinCriteriaWrongReferenceOrder = 114,
    WildcardBeforeSingleTargets = 115,
    RequireDereference = 116,
    InvalidIdentifierCharacter = 117,
    UnnecessaryQuoting = 118,
    UnnecessaryElse = 119,
    UnnecessaryCase = 120,
    UnreferencedCTE = 121,
    NestedCase = 122,
    DistinctUsedWithParentheses = 123,
    BlockedWord = 124,
    KeywordUsedAsIdentifier = 125,
    ImplicitCoerced = 126,
    LinterError = 127,
    SubstitutionRequiresTableAlias = 128,
    ColumnNameCaseMismatch = 129,
    DuplicatedColumnName = 130,

    /// Generic catch-all error for all errors originating from the PySpark LP
    /// binder. The PySpark LP binder is different in that its input is a
    /// serialized Spark logical plan that is supposedly well-formed, so we
    /// don't bother with fine-grained error codes.
    PySparkError = 199,

    // ----------------- Semantic errors -----------------
    NameNotFound = 201,
    AmbiguousName = 202,
    InvalidUnnest = 203,
    InvalidLiteral = 204,
    InvalidEscape = 205,
    InvalidInterval = 206,
    InvalidProperty = 207,
    UnknownVariable = 208,
    FunctionResolutionFailed = 209,
    TableFunctionResolutionFailed = 210,
    SchemaMismatch = 211,
    DuplicateCteName = 212,
    UnaggregatedColumn = 213,
    TableNotFound = 214,
    TableWildcardNotFound = 215,
    InvalidGroupByOrdinal = 216,
    InvalidAlias = 217,
    InvalidHavingClause = 218,
    UnresolvedColumnOrdinal = 219,
    ColumnAliasMismatch = 220,
    InvalidTableName = 221,
    InvalidSetOperation = 222,
    InvalidPatternRecognition = 223,
    InvalidJoinCriteria = 224,
    TableMissingLocation = 225,
    NonUniformTypeArray = 226,
    UnresolvedIdentifier = 227,
    IncompatibleTypes = 228,
    DuplicateProperty = 229,
    IllegalType = 230,
    InvalidTimeUnit = 231,
    UnresolvedRegex = 232,
    UnresolvedGroupBy = 233,
    UnresolvedWindow = 234,
    InvalidStruct = 235,
    UnknownFunctionLanguage = 236,
    UnresolvedStage = 237,
    InvalidBooleanExpression = 238,
    InvalidPartitionBy = 239,
    InvalidStageName = 240,
    SnowflakeNotSupportTimeUnitAfterInterval = 241,
    ImplicitColumnAlias = 242,
    InvalidSchemaName = 243,
    UnknownType = 244,
    CatalogNotFound = 245,
    SchemaNotFound = 246,
    InvalidSchema = 247,
    AsteriskNeedsInput = 248,
    InvalidGroupByClause = 249,
    InvalidDelimiter = 250,
    InvalidCte = 251,
    InvalidSubquery = 252,
    InvalidCoalesceArgument = 253,
    WindowFunctionInWrongPlace = 254,
    UnsupportedFeature = 255,
    ArgumentShouldBeStringLiteral = 256,
    InvalidDatabricksNumberFormat = 257,

    // ----------------- Coarse grained errors -----------------
    BadQueryLimitTarget = 401,
    BadRowCount = 402,
    BadInlineTable = 403,
    BadQuery = 404,
    BadTableFunctionArgument = 405,
    InvalidPredicate = 406,
    InvalidComparison = 407,
    InvalidBetween = 408,
    InvalidInList = 409,
    InvalidInSubquery = 410,
    InvalidArithmetic = 411,
    InvalidConcatenation = 412,
    InvalidValueExpression = 413,
    InvalidListAgg = 414,
    InvalidCase = 415,
    InvalidCast = 416,
    InvalidFunctionCall = 417,
    InvalidSubscript = 418,
    InvalidDereference = 419,
    InvalidJsonExists = 420,
    InvalidJsonValue = 421,
    InvalidJsonQuery = 422,
    InvalidPrimaryExpression = 423,
    InvalidJsonObject = 424,
    InvalidQualifiedName = 425,
    InvalidJsonArray = 426,
    InvalidColumnReference = 427,
    InvalidSqlFunction = 428,
    InvalidDeclare = 429,
    InvalidNamedArgument = 430,
    InvalidInsertInto = 431,
    InvalidPivot = 432,
    InvalidExpression = 433,
    InvalidCollate = 434,
    InvalidRegexp = 435,
    InvalidFunctionName = 436,
    BadLateralView = 437,
    InvalidNamedStruct = 438,
    ErrorInCustomFunctionBinder = 439,
    InvalidSampleSpecification = 440,
    // ----------------- PlanBuilder errors -----------------
    ProjectionFailed = 301,
    JoinFailed = 302,
    AggregateFailed = 303,
    SetOperationFailed = 304,
    SortFailed = 305,
    LimitFailed = 306,
    CreateViewFailed = 307,
    CreateTableFailed = 308,
    CreateIndexFailed = 309,
    InsertIntoFailed = 310,
    DistinctFailed = 311,
    ValuesFailed = 312,
    HavingFailed = 313,
    WindowFailed = 314,
    TableAliasFailed = 315,
    ScanFailed = 316,
    FilterFailed = 317,
    RecursiveCTEFailed = 318,
    QualifyFailed = 319,

    // ----------------- Legacy errors (for transition only) -----------------
    LegacyBinder = 892,
    LegacyParser = 893,
    LegacyDatafusion = 894,

    // ----------------- Internal errors -----------------
    // Everything below this line is an internal error. They will be presented
    // as bugs if surfaced to the user.
    NotSupported = 900,
    Unknown = 901,
    Unexpected = 902,
    NotImplemented = 903,
    // This is Antlr internal failure, different from SyntaxError, usually
    // indicates a bug in the grammar
    AntlrError = 904,
    ExecutionError = 905,
    JinjaError = 906,
    JinjaAssertionError = 907,
}

impl Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:03}", *self as u16)
    }
}

impl ErrorCode {
    pub fn is_bug(&self) -> bool {
        (*self as u16) >= (Self::NotSupported as u16)
    }
}
