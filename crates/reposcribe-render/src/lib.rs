use std::{
    fs,
    path::{Path, PathBuf},
};

use reposcribe_core::{DatabaseSchema, FlowDiagram, OutputFormat, SequenceDiagram};
use thiserror::Error;

pub fn render_erd(
    schema: &DatabaseSchema,
    format: OutputFormat,
    destination: &Path,
) -> Result<PathBuf, RenderError> {
    let mermaid = render_erd_mermaid(schema);
    let svg =
        mermaid_svg::render(&mermaid).map_err(|error| RenderError::Mermaid(error.to_string()))?;
    let markdown = format!(
        "# {}\n\n```mermaid\n{}\n```\n\n## Sources\n\n{}",
        schema.name,
        mermaid,
        schema
            .source_files
            .iter()
            .map(|source| format!("- `{}`", source.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let html = render_html(
        &schema.name,
        &format!(
            "{} entities · {} relationships",
            schema.entities.len(),
            schema.relationships.len()
        ),
        &svg,
    );
    write_diagram(format, destination, markdown, html, svg)
}

pub fn render_sequence(
    diagram: &SequenceDiagram,
    format: OutputFormat,
    destination: &Path,
) -> Result<PathBuf, RenderError> {
    render_mermaid(
        &diagram.name,
        &diagram.entry,
        &diagram.source_files,
        &diagram.mermaid,
        format,
        destination,
    )
}

pub fn render_flow(
    diagram: &FlowDiagram,
    format: OutputFormat,
    destination: &Path,
) -> Result<PathBuf, RenderError> {
    render_mermaid(
        &diagram.name,
        &diagram.entry,
        &diagram.source_files,
        &diagram.mermaid,
        format,
        destination,
    )
}

fn render_mermaid(
    name: &str,
    entry: &str,
    source_files: &[PathBuf],
    mermaid: &str,
    format: OutputFormat,
    destination: &Path,
) -> Result<PathBuf, RenderError> {
    let mermaid = mermaid.trim();
    let svg =
        mermaid_svg::render(mermaid).map_err(|error| RenderError::Mermaid(error.to_string()))?;
    let markdown = mermaid_markdown(name, entry, source_files, mermaid);
    let html = render_html(name, &format!("Entry: {entry}"), &svg);
    write_diagram(format, destination, markdown, html, svg)
}

fn write_diagram(
    format: OutputFormat,
    destination: &Path,
    markdown: String,
    html: String,
    svg: String,
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
        OutputFormat::Markdown => markdown.into_bytes(),
        OutputFormat::Html => html.into_bytes(),
        OutputFormat::Pdf => svg_to_pdf(&svg)?,
    };
    fs::write(&path, bytes).map_err(|source| RenderError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn mermaid_markdown(name: &str, entry: &str, sources: &[PathBuf], mermaid: &str) -> String {
    let mut output =
        format!("# {name}\n\nEntry: `{entry}`\n\n```mermaid\n{mermaid}\n```\n\n## Sources\n\n");
    for source in sources {
        output.push_str(&format!("- `{}`\n", source.display()));
    }
    output
}

pub fn render_erd_mermaid(schema: &DatabaseSchema) -> String {
    let mut output = "erDiagram\n".to_owned();
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
            relationship.from_cardinality.mermaid_left(),
            relationship.to_cardinality.mermaid_right(),
            mermaid_identifier(&relationship.to_entity),
            mermaid_label(relationship.label.as_deref().unwrap_or("relates to"))
        ));
    }
    output
}

fn render_html(title: &str, summary: &str, svg: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>body{{margin:0;background:#f5f7fb;color:#172033;font-family:Inter,-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}}main{{max-width:1600px;margin:0 auto;padding:40px}}h1{{margin:0 0 8px}}p{{margin:0 0 28px;color:#65708a}}.diagram{{overflow:auto;background:white;border:1px solid #e3e8f2;border-radius:16px;padding:20px;box-shadow:0 12px 30px rgba(40,55,90,.08)}}svg{{display:block;max-width:none}}</style></head><body><main><h1>{}</h1><p>{} · generated by RepoScribe</p><div class=\"diagram\">{svg}</div></main></body></html>",
        escape_xml(title),
        escape_xml(title),
        escape_xml(summary)
    )
}

fn svg_to_pdf(svg: &str) -> Result<Vec<u8>, RenderError> {
    let mut options = svg2pdf::usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = svg2pdf::usvg::Tree::from_str(svg, &options)
        .map_err(|error| RenderError::Svg(error.to_string()))?;
    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|error| RenderError::Pdf(error.to_string()))
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

fn mermaid_label(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace(['\"', '\\'], "'")
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
    #[error("invalid or unsupported Mermaid diagram: {0}")]
    Mermaid(String),
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
    use reposcribe_core::{
        Cardinality, DatabaseEntity, DatabaseField, DatabaseRelationship, FlowDiagram,
        SequenceDiagram,
    };

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
        }
    }

    #[test]
    fn renders_every_erd_cardinality_on_the_correct_side() {
        let cardinalities = [
            Cardinality::One,
            Cardinality::ZeroOrOne,
            Cardinality::Many,
            Cardinality::ZeroOrMany,
        ];
        for from in cardinalities {
            for to in cardinalities {
                let mut schema = schema();
                schema.relationships[0].from_cardinality = from;
                schema.relationships[0].to_cardinality = to;
                let mermaid = render_erd_mermaid(&schema);

                mermaid_svg::render(&mermaid).unwrap_or_else(|error| {
                    panic!("failed to render {from:?} to {to:?}: {error}\n{mermaid}")
                });
            }
        }

        let mut many_to_many = schema();
        many_to_many.relationships[0].from_cardinality = Cardinality::Many;
        many_to_many.relationships[0].to_cardinality = Cardinality::Many;
        assert!(render_erd_mermaid(&many_to_many).contains("User }|--|{ Post"));
    }

    #[test]
    fn renders_detailed_mermaid_sequence_and_flow_in_all_formats() {
        let directory = tempfile::tempdir().unwrap();
        let sequence = SequenceDiagram {
            name: "Create order".to_owned(),
            entry: "POST /orders".to_owned(),
            source_files: vec![PathBuf::from("src/orders.rs")],
            mermaid: "sequenceDiagram\n  autonumber\n  participant C as Client\n  participant A as OrdersController.create_order()\n  C->>A: POST /orders\n  activate A\n  A-->>C: OrderResponse\n  deactivate A".to_owned(),
        };
        let flow = FlowDiagram {
            name: "Create order".to_owned(),
            entry: "create_order".to_owned(),
            source_files: vec![PathBuf::from("src/orders.rs")],
            mermaid: "flowchart TD\n  A[create_order()] --> B{validate_order()}\n  B -->|valid| C[repository.save(order)]\n  B -->|invalid| D[return ValidationError]".to_owned(),
        };
        for format in OutputFormat::ALL {
            assert!(
                render_sequence(&sequence, format, &directory.path().join("sequence"))
                    .unwrap()
                    .is_file()
            );
            assert!(
                render_flow(&flow, format, &directory.path().join("flow"))
                    .unwrap()
                    .is_file()
            );
        }
        let markdown = fs::read_to_string(directory.path().join("sequence.md")).unwrap();
        assert!(markdown.contains("OrdersController.create_order()"));
        assert!(markdown.contains("```mermaid"));
    }
}
