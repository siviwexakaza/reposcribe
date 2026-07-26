use std::{
    fs,
    path::{Path, PathBuf},
};

use reposcribe_core::{DatabaseEntity, DatabaseSchema, OutputFormat};
use thiserror::Error;

pub fn render_erd(
    schema: &DatabaseSchema,
    format: OutputFormat,
    destination: &Path,
) -> Result<PathBuf, RenderError> {
    let path = destination.with_extension(format.extension());
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| RenderError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let bytes = match format {
        OutputFormat::Markdown => render_markdown(schema).into_bytes(),
        OutputFormat::Html => render_html(schema).into_bytes(),
        OutputFormat::Pdf => render_pdf(schema)?,
    };
    fs::write(&path, bytes).map_err(|source| RenderError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

pub fn render_markdown(schema: &DatabaseSchema) -> String {
    let mut output = format!("# {}\n\n```mermaid\nerDiagram\n", schema.name);
    for entity in &schema.entities {
        output.push_str(&format!("  {} {{\n", mermaid_identifier(&entity.name)));
        for field in &entity.fields {
            let mut flags = Vec::new();
            if field.primary_key {
                flags.push("PK");
            }
            if field.unique {
                flags.push("UK");
            }
            output.push_str(&format!(
                "    {} {}{}\n",
                mermaid_identifier(&field.data_type),
                mermaid_identifier(&field.name),
                if flags.is_empty() {
                    String::new()
                } else {
                    format!(" {}", flags.join(","))
                }
            ));
        }
        output.push_str("  }\n");
    }
    for relationship in &schema.relationships {
        output.push_str(&format!(
            "  {} {}--{} {} : \"{}\"\n",
            mermaid_identifier(&relationship.from_entity),
            relationship.from_cardinality.mermaid(),
            relationship.to_cardinality.mermaid(),
            mermaid_identifier(&relationship.to_entity),
            relationship.label.as_deref().unwrap_or("relates to")
        ));
    }
    output.push_str("```\n\n## Sources\n\n");
    for source in &schema.source_files {
        output.push_str(&format!("- `{}`\n", source.display()));
    }
    output
}

pub fn render_html(schema: &DatabaseSchema) -> String {
    let svg = render_svg(schema);
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{margin:0;background:#f5f7fb;color:#172033;font-family:Inter,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}}main{{max-width:1500px;margin:0 auto;padding:40px}}h1{{margin:0 0 8px}}p{{margin:0 0 28px;color:#65708a}}.diagram{{overflow:auto;background:white;border:1px solid #e3e8f2;border-radius:16px;padding:20px;box-shadow:0 12px 30px rgba(40,55,90,.08)}}svg{{display:block;max-width:none}}</style></head><body><main><h1>{}</h1><p>{} entities · {} relationships · generated locally by RepoScribe</p><div class=\"diagram\">{}</div></main></body></html>",
        escape_xml(&schema.name),
        escape_xml(&schema.name),
        schema.entities.len(),
        schema.relationships.len(),
        svg
    )
}

fn render_pdf(schema: &DatabaseSchema) -> Result<Vec<u8>, RenderError> {
    let svg = render_svg(schema);
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = svg2pdf::usvg::Tree::from_str(&svg, &options)
        .map_err(|error| RenderError::Svg(error.to_string()))?;
    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|error| RenderError::Pdf(error.to_string()))
}

fn render_svg(schema: &DatabaseSchema) -> String {
    const CARD_WIDTH: f32 = 280.0;
    const HEADER_HEIGHT: f32 = 42.0;
    const ROW_HEIGHT: f32 = 24.0;
    const GAP_X: f32 = 70.0;
    const GAP_Y: f32 = 70.0;
    const MARGIN: f32 = 40.0;

    let count = schema.entities.len().max(1);
    let columns = (count as f32).sqrt().ceil() as usize;
    let rows = count.div_ceil(columns);
    let card_heights: Vec<f32> = schema
        .entities
        .iter()
        .map(|entity| HEADER_HEIGHT + ROW_HEIGHT * entity.fields.len().max(1) as f32 + 14.0)
        .collect();
    let max_height = card_heights.iter().copied().fold(90.0, f32::max);
    let width = MARGIN * 2.0 + columns as f32 * CARD_WIDTH + (columns - 1) as f32 * GAP_X;
    let height = MARGIN * 2.0 + 48.0 + rows as f32 * max_height + (rows - 1) as f32 * GAP_Y;

    let positions: Vec<(f32, f32)> = schema
        .entities
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let column = index % columns;
            let row = index / columns;
            (
                MARGIN + column as f32 * (CARD_WIDTH + GAP_X),
                MARGIN + 48.0 + row as f32 * (max_height + GAP_Y),
            )
        })
        .collect();

    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\"><rect width=\"100%\" height=\"100%\" fill=\"#f8fafc\"/><defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"8\" refY=\"5\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto-start-reverse\"><path d=\"M 0 0 L 10 5 L 0 10 z\" fill=\"#7c8aa5\"/></marker></defs><text x=\"{MARGIN}\" y=\"32\" font-family=\"sans-serif\" font-size=\"22\" font-weight=\"700\" fill=\"#172033\">{}</text>",
        escape_xml(&schema.name)
    );

    for relationship in &schema.relationships {
        let Some(from_index) = schema
            .entities
            .iter()
            .position(|entity| entity.name == relationship.from_entity)
        else {
            continue;
        };
        let Some(to_index) = schema
            .entities
            .iter()
            .position(|entity| entity.name == relationship.to_entity)
        else {
            continue;
        };
        let (from_x, from_y) = positions[from_index];
        let (to_x, to_y) = positions[to_index];
        let start_x = from_x + CARD_WIDTH / 2.0;
        let start_y = from_y + card_heights[from_index] / 2.0;
        let end_x = to_x + CARD_WIDTH / 2.0;
        let end_y = to_y + card_heights[to_index] / 2.0;
        svg.push_str(&format!(
            "<path d=\"M {start_x} {start_y} L {end_x} {end_y}\" stroke=\"#7c8aa5\" stroke-width=\"2\" fill=\"none\" marker-end=\"url(#arrow)\"/>"
        ));
    }

    for (index, entity) in schema.entities.iter().enumerate() {
        let (x, y) = positions[index];
        svg.push_str(&render_entity_card(
            entity,
            x,
            y,
            CARD_WIDTH,
            card_heights[index],
        ));
    }
    svg.push_str("</svg>");
    svg
}

fn render_entity_card(entity: &DatabaseEntity, x: f32, y: f32, width: f32, height: f32) -> String {
    let mut svg = format!(
        "<g><rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" rx=\"10\" fill=\"white\" stroke=\"#ccd5e5\" stroke-width=\"1.5\"/><rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"42\" rx=\"10\" fill=\"#4f46e5\"/><text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"16\" font-weight=\"700\" fill=\"white\">{}</text>",
        x + 14.0,
        y + 27.0,
        escape_xml(&entity.name)
    );
    for (index, field) in entity.fields.iter().enumerate() {
        let field_y = y + 64.0 + index as f32 * 24.0;
        let flags = match (field.primary_key, field.unique, field.nullable) {
            (true, _, _) => "PK",
            (_, true, _) => "UK",
            (_, _, true) => "?",
            _ => "",
        };
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{field_y}\" font-family=\"monospace\" font-size=\"12\" fill=\"#27324a\">{}</text><text x=\"{}\" y=\"{field_y}\" text-anchor=\"end\" font-family=\"monospace\" font-size=\"11\" fill=\"#7a859c\">{} {}</text>",
            x + 14.0,
            escape_xml(&field.name),
            x + width - 14.0,
            escape_xml(&field.data_type),
            flags
        ));
    }
    svg.push_str("</g>");
    svg
}

fn mermaid_identifier(value: &str) -> String {
    let value: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if value.is_empty() {
        "unknown".to_owned()
    } else {
        value
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("could not create SVG: {0}")]
    Svg(String),
    #[error("could not create PDF: {0}")]
    Pdf(String),
    #[error("could not write '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use reposcribe_core::{Cardinality, DatabaseField, DatabaseRelationship};

    use super::*;

    fn schema() -> DatabaseSchema {
        DatabaseSchema {
            name: "Example schema".to_owned(),
            source_files: vec![PathBuf::from("schema.prisma")],
            entities: vec![
                DatabaseEntity {
                    name: "User".to_owned(),
                    fields: vec![DatabaseField {
                        name: "id".to_owned(),
                        data_type: "String".to_owned(),
                        nullable: false,
                        primary_key: true,
                        unique: true,
                    }],
                },
                DatabaseEntity {
                    name: "Post".to_owned(),
                    fields: vec![DatabaseField {
                        name: "id".to_owned(),
                        data_type: "String".to_owned(),
                        nullable: false,
                        primary_key: true,
                        unique: true,
                    }],
                },
            ],
            relationships: vec![DatabaseRelationship {
                from_entity: "User".to_owned(),
                from_field: None,
                to_entity: "Post".to_owned(),
                to_field: None,
                from_cardinality: Cardinality::One,
                to_cardinality: Cardinality::ZeroOrMany,
                label: Some("posts".to_owned()),
            }],
        }
    }

    #[test]
    fn renders_all_public_output_formats() {
        let directory = tempfile::tempdir().unwrap();
        for format in OutputFormat::ALL {
            let path = render_erd(&schema(), format, &directory.path().join("erd")).unwrap();
            assert!(path.is_file());
            assert!(fs::metadata(path).unwrap().len() > 20);
        }
    }
}
