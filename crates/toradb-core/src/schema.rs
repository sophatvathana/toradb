pub type DocId = u64;
pub type SegmentId = u32;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    pub columns: Vec<ColumnDef>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub kind: ColumnKind,
    #[serde(default)]
    pub column_type: ColumnType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColumnKind {
    Uuid,
    Text,
    Int,
    Vector,
    Graph,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ColumnType {
    #[default]
    Text,
    Int,
    Float,
    Bool,
    Date,
    Timestamp,
    Json,
    Uuid,
    Vector,
}

impl ColumnType {
    pub fn parse(s: &str) -> ColumnType {
        let head = s.trim().split('(').next().unwrap_or("").trim();
        match head.to_ascii_uppercase().as_str() {
            "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "TINYINT" => ColumnType::Int,
            "FLOAT" | "DOUBLE" | "REAL" | "DECIMAL" | "NUMERIC" => ColumnType::Float,
            "BOOL" | "BOOLEAN" => ColumnType::Bool,
            "DATE" => ColumnType::Date,
            "TIMESTAMP" | "DATETIME" => ColumnType::Timestamp,
            "JSON" | "JSONB" => ColumnType::Json,
            "UUID" => ColumnType::Uuid,
            "VECTOR" => ColumnType::Vector,
            // "TEXT" | "VARCHAR" | "CHAR" | "STRING" and anything unrecognized.
            _ => ColumnType::Text,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ColumnType::Text => "text",
            ColumnType::Int => "int",
            ColumnType::Float => "float",
            ColumnType::Bool => "bool",
            ColumnType::Date => "date",
            ColumnType::Timestamp => "timestamp",
            ColumnType::Json => "json",
            ColumnType::Uuid => "uuid",
            ColumnType::Vector => "vector",
        }
    }
}
