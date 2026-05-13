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
pub enum SelectExpr {
    Column(String),
    CountStar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WhereEq {
    pub column: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectStmt {
    pub table: String,
    pub select_items: Vec<SelectExpr>,
    /// BM25 / sparse method name (e.g. "bm25").
    pub sparse: Option<String>,
    /// Query text from BM25('...') or SPARSE SEARCH clause.
    pub sparse_query: Option<String>,
    pub vector: bool,
    pub limit: u32,
    pub group_by: Option<String>,
    pub where_eq: Option<WhereEq>,
}
