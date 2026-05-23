#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    CreateTable(CreateTableStmt),
    CreateIndex(CreateIndexStmt),
    DropTable { name: String },
    ShowTables,
    Describe { name: String },
    Select(SelectStmt),
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
pub struct SelectStmt {
    pub table: String,
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
    pub group_by: Option<String>,
    pub where_clause: Option<WherePred>,
}
