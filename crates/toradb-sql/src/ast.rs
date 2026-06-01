#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    CreateTable(CreateTableStmt),
    CreateIndex(CreateIndexStmt),
    CreateMaterializedView(CreateMaterializedViewStmt),
    RefreshMaterializedView { name: String },
    DropMaterializedView { name: String },
    AlterTableSetSegmentWorkers { table: String, workers: u32 },
    AlterTableAlterColumnType {
        table: String,
        column: String,
        column_type: String,
        /// When true, run `COMPACT TABLE … FULL` after updating the manifest.
        rewrite: bool,
    },
    CompactTable { table: String, full: bool },
    DropTable { name: String },
    ShowTables,
    ShowMaterializedViews,
    ShowIndexes { table: String },
    ShowCreateTable { table: String },
    Describe { name: String },
    Select(SelectStmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateMaterializedViewStmt {
    pub name: String,
    pub select: SelectStmt,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateTableStmt {
    /// Optional namespace prefix (`db.table` → namespace + table).
    pub namespace: Option<String>,
    pub name: String,
    pub mode: String,
    pub columns: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndexStmt {
    pub name: String,
    pub namespace: Option<String>,
    pub table: String,
    pub column: String,
    pub using: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggFunc {
    CountStar,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectExpr {
    All,
    Column(String),
    Aggregate {
        func: AggFunc,
        column: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WherePred {
    Compare {
        column: String,
        op: CompareOp,
        value: String,
    },
    In {
        column: String,
        values: Vec<String>,
    },
    Between {
        column: String,
        low: String,
        high: String,
        negated: bool,
    },
    Like {
        column: String,
        pattern: String,
        negated: bool,
    },
    And(Vec<WherePred>),
    Or(Vec<WherePred>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub column: String,
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinClause {
    pub right_table: String,
    pub left_key: String,
    pub right_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cte {
    pub name: String,
    pub query: Box<SelectStmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStmt {
    pub ctes: Vec<Cte>,
    pub table: String,
    pub join: Option<JoinClause>,
    pub select_items: Vec<SelectExpr>,
    /// When true, dedupe projected rows (`SELECT DISTINCT ...`).
    pub distinct: bool,
    /// BM25 / sparse method name (e.g. "bm25").
    pub sparse: Option<String>,
    /// Query text from BM25('...') or SPARSE SEARCH clause.
    pub sparse_query: Option<String>,
    /// True when a VECTOR SEARCH clause is present.
    pub vector: bool,
    /// Literal query embedding from ANN([...]).
    pub vector_query: Option<Vec<f32>>,
    /// Text query for lexical proxy embedding from ANN('...').
    pub vector_text: Option<String>,
    pub limit: u32,
    pub offset: u32,
    /// `ORDER BY <column> [ASC|DESC]`; `None` = retrieval merge order. A `column` of
    /// `"score"` orders by relevance (the original behavior).
    pub order_by: Option<OrderBy>,
    /// When true, scan segment shards in parallel (single-node or cluster distributed execution).
    pub distributed: bool,
    pub hyde: bool,
    pub crag: bool,
    pub graph_expand: bool,
    pub graph_depth: u32,
    /// RRF fusion constant (default 60).
    pub fusion_k: u32,
    /// When true, clients should page results (see `Database.sql_stream`).
    pub stream: bool,
    /// When true, return a plan only (`EXPLAIN`); do not execute retrieval.
    pub explain: bool,
    pub group_by: Vec<String>,
    pub where_clause: Option<WherePred>,
    pub having_clause: Option<WherePred>,
}
