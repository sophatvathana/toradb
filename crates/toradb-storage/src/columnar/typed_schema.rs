use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use toradb_core::{ColumnType, ColumnTypeSpec};

use super::schema::doc_schema;

/// Parquet/Arrow schema with native typed metadata columns.
pub fn table_doc_schema(column_types: &[(String, ColumnTypeSpec)]) -> Arc<Schema> {
    if column_types.is_empty() {
        return doc_schema();
    }
    let mut fields = vec![
        Field::new("id", DataType::UInt64, false),
        Field::new("text", DataType::Utf8, false),
    ];
    for (name, ty) in sorted_column_types(column_types) {
        fields.push(Field::new(
            name.as_str(),
            column_type_to_arrow(ty.kind),
            true,
        ));
    }
    fields.push(Field::new("metadata_json", DataType::Utf8, true));
    fields.push(Field::new(
        "embedding",
        DataType::List(Arc::new(Field::new("item", DataType::Float32, true))),
        true,
    ));
    Arc::new(Schema::new(fields))
}

pub fn column_type_to_arrow(ty: ColumnType) -> DataType {
    match ty {
        ColumnType::Int => DataType::Int64,
        ColumnType::Float => DataType::Float64,
        ColumnType::Bool => DataType::Boolean,
        ColumnType::Date => DataType::Date32,
        ColumnType::Timestamp => DataType::Timestamp(TimeUnit::Millisecond, None),
        ColumnType::Vector => DataType::List(Arc::new(Field::new("item", DataType::Float32, true))),
        ColumnType::Text | ColumnType::Json | ColumnType::Uuid => DataType::Utf8,
    }
}

/// Stable lowercase column order for schema and encode.
pub fn sorted_column_types(column_types: &[(String, ColumnTypeSpec)]) -> Vec<(String, ColumnTypeSpec)> {
    let mut out: Vec<_> = column_types
        .iter()
        .map(|(n, t)| (n.to_ascii_lowercase(), *t))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Legacy segment: exactly `id`, `text`, `metadata_json`, `embedding`.
pub fn is_legacy_arrow_schema(schema: &Schema) -> bool {
    if schema.fields().len() != 4 {
        return false;
    }
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    names == ["id", "text", "metadata_json", "embedding"]
}

pub fn uses_native_metadata(schema: &Schema) -> bool {
    !is_legacy_arrow_schema(schema) && schema.field_with_name("metadata_json").is_ok()
}

/// Column names to read for metadata scans: `id` + typed fields + `metadata_json`.
pub fn metadata_scan_column_names(column_types: &[(String, ColumnTypeSpec)]) -> Vec<String> {
    let mut names = vec!["id".to_string()];
    for (n, _) in sorted_column_types(column_types) {
        names.push(n);
    }
    names.push("metadata_json".to_string());
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_schema_includes_typed_fields() {
        let schema = table_doc_schema(&[("rank".into(), ColumnTypeSpec::new(ColumnType::Int))]);
        let names: Vec<_> = schema
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .collect();
        assert_eq!(
            names,
            vec!["id", "text", "rank", "metadata_json", "embedding"]
        );
    }
}
