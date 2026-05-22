#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    CreateTable(CreateTableStmt),
    CreateIndex(CreateIndexStmt),
    CreateMaterializedView(CreateMaterializedViewStmt),
    RefreshMaterializedView { name: String },
    DropMaterializedView { name: String },
    DropTable { name: String },
    ShowTables,
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
    pub name: String,
    pub mode: String,
    pub columns: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndexStmt {
    pub name: String,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct JoinClause {
    pub right_table: String,
    pub left_key: String,
    pub right_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStmt {
    pub table: String,
    pub join: Option<JoinClause>,
    pub select_items: Vec<SelectExpr>,
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
    /// `Some(true)` = ORDER BY score DESC, `Some(false)` = ASC, `None` = retrieval merge order.
    pub order_by_score_desc: Option<bool>,
    /// When true, scan segment shards in parallel (single-node distributed execution).
    pub distributed: bool,
    /// When true, clients should page results (see `Database.sql_stream`).
    pub stream: bool,
    pub group_by: Option<String>,
    pub where_clause: Option<WherePred>,
}
