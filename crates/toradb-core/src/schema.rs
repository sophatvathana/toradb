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
        parse_base_type(s)
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

fn parse_base_type(s: &str) -> ColumnType {
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
        _ => ColumnType::Text,
    }
}

/// Full column type including optional vector dimension (e.g. `vector(384)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnTypeSpec {
    pub kind: ColumnType,
    pub vector_dim: Option<u32>,
}

impl ColumnTypeSpec {
    pub fn new(kind: ColumnType) -> Self {
        Self {
            kind,
            vector_dim: None,
        }
    }

    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if let Some(dim) = parse_vector_type_dim(s) {
            return Self {
                kind: ColumnType::Vector,
                vector_dim: dim,
            };
        }
        Self {
            kind: parse_base_type(s),
            vector_dim: None,
        }
    }

    pub fn from_manifest_str(s: &str) -> Self {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("vector:") {
            let dim = rest.parse().ok();
            return Self {
                kind: ColumnType::Vector,
                vector_dim: dim,
            };
        }
        if s == "vector" {
            return Self::new(ColumnType::Vector);
        }
        Self::parse(s)
    }

    pub fn sql_name(self) -> String {
        match (self.kind, self.vector_dim) {
            (ColumnType::Vector, Some(d)) => format!("vector({d})"),
            _ => self.kind.as_str().to_string(),
        }
    }

    pub fn manifest_str(self) -> String {
        match (self.kind, self.vector_dim) {
            (ColumnType::Vector, Some(d)) => format!("vector:{d}"),
            _ => self.kind.as_str().to_string(),
        }
    }

    pub fn kind(self) -> ColumnType {
        self.kind
    }
}

impl Default for ColumnTypeSpec {
    fn default() -> Self {
        Self::new(ColumnType::Text)
    }
}

impl serde::Serialize for ColumnTypeSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.manifest_str())
    }
}

impl<'de> serde::Deserialize<'de> for ColumnTypeSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_manifest_str(&s))
    }
}

fn parse_vector_type_dim(s: &str) -> Option<Option<u32>> {
    let upper = s.to_ascii_uppercase();
    if !upper.starts_with("VECTOR") {
        return None;
    }
    let rest = s[6..].trim_start();
    if rest.is_empty() {
        return Some(None);
    }
    let inner = rest
        .strip_prefix('(')
        .or_else(|| rest.strip_prefix('['))
        .and_then(|r| r.strip_suffix(')').or_else(|| r.strip_suffix(']')));
    Some(inner.and_then(|n| n.trim().parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vector_with_paren_dim() {
        let spec = ColumnTypeSpec::parse("vector(384)");
        assert_eq!(spec.kind, ColumnType::Vector);
        assert_eq!(spec.vector_dim, Some(384));
        assert_eq!(spec.sql_name(), "vector(384)");
        assert_eq!(spec.manifest_str(), "vector:384");
    }

    #[test]
    fn parse_vector_with_bracket_dim() {
        let spec = ColumnTypeSpec::parse("vector[768]");
        assert_eq!(spec.vector_dim, Some(768));
    }

    #[test]
    fn manifest_vector_dim_round_trip_via_strings() {
        let spec = ColumnTypeSpec::parse("vector(384)");
        assert_eq!(spec.manifest_str(), "vector:384");
        let back = ColumnTypeSpec::from_manifest_str(&spec.manifest_str());
        assert_eq!(back, spec);
    }

    #[test]
    fn legacy_manifest_int_still_loads() {
        let back = ColumnTypeSpec::from_manifest_str("int");
        assert_eq!(back.kind, ColumnType::Int);
        assert_eq!(back.vector_dim, None);
    }
}
