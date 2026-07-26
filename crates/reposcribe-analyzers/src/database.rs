use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use ignore::WalkBuilder;
use reposcribe_core::{
    Cardinality, DatabaseEntity, DatabaseField, DatabaseRelationship, DatabaseSchema,
    ProjectProfile,
};

use crate::DetectionError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DatabaseSourceKind {
    Prisma,
    Rails,
    Sql,
}

impl std::fmt::Display for DatabaseSourceKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Prisma => "Prisma",
            Self::Rails => "Rails schema",
            Self::Sql => "SQL",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSource {
    pub project_name: String,
    pub project_root: PathBuf,
    pub kind: DatabaseSourceKind,
    pub files: Vec<PathBuf>,
}

impl std::fmt::Display for DatabaseSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} · {} · {} file{}",
            self.project_name,
            self.kind,
            self.files.len(),
            if self.files.len() == 1 { "" } else { "s" }
        )
    }
}

pub fn discover_database_sources(
    profile: &ProjectProfile,
) -> Result<Vec<DatabaseSource>, DetectionError> {
    let mut groups: BTreeMap<(PathBuf, DatabaseSourceKind), Vec<PathBuf>> = BTreeMap::new();
    let walker = WalkBuilder::new(&profile.repository_root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_string_lossy().as_ref(),
                ".git" | ".reposcribe" | "node_modules" | "target" | "vendor" | "dist" | "build"
            )
        })
        .build();

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&profile.repository_root)
            .unwrap_or(entry.path())
            .to_path_buf();
        let Some(kind) = classify_database_file(&relative) else {
            continue;
        };
        let module_root = closest_module_root(profile, &relative);
        groups
            .entry((module_root, kind))
            .or_default()
            .push(relative);
    }

    let mut sources: Vec<DatabaseSource> = groups
        .into_iter()
        .map(|((project_root, kind), mut files)| {
            files.sort();
            let project_name = profile
                .modules
                .iter()
                .find(|module| module.root == project_root)
                .map(|module| module.name.clone())
                .unwrap_or_else(|| {
                    project_root
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("repository")
                        .to_owned()
                });
            DatabaseSource {
                project_name,
                project_root,
                kind,
                files,
            }
        })
        .collect();
    sources.sort_by(|left, right| {
        (&left.project_root, left.kind).cmp(&(&right.project_root, right.kind))
    });
    Ok(sources)
}

pub fn parse_database_source(
    repository_root: &Path,
    source: &DatabaseSource,
) -> Result<DatabaseSchema, DatabaseParseError> {
    let mut contents = String::new();
    for file in &source.files {
        let absolute = repository_root.join(file);
        let value = fs::read_to_string(&absolute).map_err(|error| DatabaseParseError::Read {
            path: absolute,
            error,
        })?;
        contents.push_str(&value);
        contents.push_str("\n\n");
    }

    let mut schema = match source.kind {
        DatabaseSourceKind::Prisma => parse_prisma(&contents),
        DatabaseSourceKind::Rails => parse_rails_schema(&contents),
        DatabaseSourceKind::Sql => parse_sql(&contents),
    };
    if schema.entities.is_empty() {
        return Err(DatabaseParseError::NoEntities(source.kind));
    }
    schema.name = format!("{} database", source.project_name);
    schema.source_files = source.files.clone();
    Ok(schema)
}

fn classify_database_file(path: &Path) -> Option<DatabaseSourceKind> {
    let normalized = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if normalized.ends_with("schema.prisma") {
        return Some(DatabaseSourceKind::Prisma);
    }
    if normalized.ends_with("db/schema.rb") {
        return Some(DatabaseSourceKind::Rails);
    }
    if normalized.ends_with(".sql")
        && ["migration", "migrations", "schema", "supabase"]
            .iter()
            .any(|hint| normalized.contains(hint))
    {
        return Some(DatabaseSourceKind::Sql);
    }
    None
}

fn closest_module_root(profile: &ProjectProfile, file: &Path) -> PathBuf {
    profile
        .modules
        .iter()
        .filter(|module| module.root.as_os_str().is_empty() || file.starts_with(&module.root))
        .max_by_key(|module| module.root.components().count())
        .map(|module| module.root.clone())
        .unwrap_or_default()
}

#[derive(Debug)]
struct RawPrismaField {
    name: String,
    field_type: String,
    attributes: String,
}

#[derive(Debug)]
struct RawPrismaModel {
    name: String,
    fields: Vec<RawPrismaField>,
}

fn parse_prisma(contents: &str) -> DatabaseSchema {
    let mut models = Vec::new();
    let mut current: Option<RawPrismaModel> = None;

    for source_line in contents.lines() {
        let line = source_line.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("model ") {
            if let Some(model) = current.take() {
                models.push(model);
            }
            let name = rest
                .split(|character: char| character.is_whitespace() || character == '{')
                .next()
                .unwrap_or("Unknown")
                .to_owned();
            current = Some(RawPrismaModel {
                name,
                fields: Vec::new(),
            });
            continue;
        }
        if line.starts_with('}') {
            if let Some(model) = current.take() {
                models.push(model);
            }
            continue;
        }
        let Some(model) = current.as_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(name), Some(field_type)) = (parts.next(), parts.next()) else {
            continue;
        };
        model.fields.push(RawPrismaField {
            name: name.to_owned(),
            field_type: field_type.to_owned(),
            attributes: parts.collect::<Vec<_>>().join(" "),
        });
    }
    if let Some(model) = current {
        models.push(model);
    }

    let model_names: BTreeSet<String> = models.iter().map(|model| model.name.clone()).collect();
    let entities = models
        .iter()
        .map(|model| DatabaseEntity {
            name: model.name.clone(),
            fields: model
                .fields
                .iter()
                .filter(|field| !model_names.contains(base_prisma_type(&field.field_type)))
                .map(|field| DatabaseField {
                    name: field.name.clone(),
                    data_type: base_prisma_type(&field.field_type).to_owned(),
                    nullable: field.field_type.ends_with('?'),
                    primary_key: field.attributes.contains("@id"),
                    unique: field.attributes.contains("@unique")
                        || field.attributes.contains("@id"),
                })
                .collect(),
        })
        .collect();

    let mut relationships: Vec<DatabaseRelationship> = Vec::new();
    for model in &models {
        for field in &model.fields {
            let target = base_prisma_type(&field.field_type);
            if !model_names.contains(target) || target == model.name {
                continue;
            }
            let explicit_field = bracket_value(&field.attributes, "fields:");
            let is_many = field.field_type.contains("[]");
            let relationship = DatabaseRelationship {
                from_entity: model.name.clone(),
                from_field: explicit_field,
                to_entity: target.to_owned(),
                to_field: bracket_value(&field.attributes, "references:"),
                from_cardinality: if is_many {
                    Cardinality::One
                } else {
                    Cardinality::Many
                },
                to_cardinality: if is_many {
                    Cardinality::ZeroOrMany
                } else if field.field_type.ends_with('?') {
                    Cardinality::ZeroOrOne
                } else {
                    Cardinality::One
                },
                label: Some(field.name.clone()),
            };
            let existing = relationships.iter().position(|candidate| {
                (candidate.from_entity == relationship.from_entity
                    && candidate.to_entity == relationship.to_entity)
                    || (candidate.from_entity == relationship.to_entity
                        && candidate.to_entity == relationship.from_entity)
            });
            match existing {
                Some(index)
                    if relationship.from_field.is_some()
                        && relationships[index].from_field.is_none() =>
                {
                    relationships[index] = relationship;
                }
                Some(_) => {}
                None => relationships.push(relationship),
            }
        }
    }

    DatabaseSchema {
        name: "Database schema".to_owned(),
        source_files: Vec::new(),
        entities,
        relationships,
    }
}

fn parse_rails_schema(contents: &str) -> DatabaseSchema {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();
    let mut current: Option<DatabaseEntity> = None;
    let mut unique_indexes: Vec<(String, String)> = Vec::new();

    for source_line in contents.lines() {
        let line = source_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("create_table ") {
            if let Some(entity) = current.take() {
                entities.push(entity);
            }
            let Some(name) = first_ruby_string(line) else {
                continue;
            };
            current = Some(DatabaseEntity {
                name,
                fields: vec![DatabaseField {
                    name: "id".to_owned(),
                    data_type: "bigint".to_owned(),
                    nullable: false,
                    primary_key: true,
                    unique: true,
                }],
            });
            continue;
        }
        if line == "end" {
            if let Some(entity) = current.take() {
                entities.push(entity);
            }
            continue;
        }
        if let Some(entity) = current.as_mut() {
            if line.starts_with("t.index ") {
                if line.contains("unique: true") {
                    if let Some(field) = first_ruby_string(line) {
                        unique_indexes.push((entity.name.clone(), field));
                    }
                }
                continue;
            }
            if let Some(field) = parse_rails_column(line) {
                if !entity
                    .fields
                    .iter()
                    .any(|existing| existing.name == field.name)
                {
                    entity.fields.push(field);
                }
            }
            continue;
        }
        if line.starts_with("add_foreign_key ") {
            let values = ruby_strings(line);
            if values.len() >= 2 {
                let from_field = ruby_option_string(line, "column:")
                    .unwrap_or_else(|| format!("{}_id", singularize(&values[1])));
                let to_field =
                    ruby_option_string(line, "primary_key:").unwrap_or_else(|| "id".to_owned());
                relationships.push(DatabaseRelationship {
                    from_entity: values[0].clone(),
                    from_field: Some(from_field.clone()),
                    to_entity: values[1].clone(),
                    to_field: Some(to_field),
                    from_cardinality: Cardinality::Many,
                    to_cardinality: Cardinality::One,
                    label: Some(from_field),
                });
            }
        }
    }
    if let Some(entity) = current {
        entities.push(entity);
    }
    for (entity_name, field_name) in unique_indexes {
        if let Some(field) = entities
            .iter_mut()
            .find(|entity| entity.name == entity_name)
            .and_then(|entity| {
                entity
                    .fields
                    .iter_mut()
                    .find(|field| field.name == field_name)
            })
        {
            field.unique = true;
        }
    }

    DatabaseSchema {
        name: "Database schema".to_owned(),
        source_files: Vec::new(),
        entities,
        relationships,
    }
}

fn parse_rails_column(line: &str) -> Option<DatabaseField> {
    let rest = line.strip_prefix("t.")?;
    let data_type = rest
        .split(|character: char| character.is_whitespace() || character == '(')
        .next()?;
    if matches!(data_type, "index" | "timestamps") {
        return None;
    }
    let name = first_ruby_string(line)?;
    let is_reference = matches!(data_type, "references" | "belongs_to");
    Some(DatabaseField {
        name: if is_reference {
            format!("{name}_id")
        } else {
            name
        },
        data_type: if is_reference { "bigint" } else { data_type }.to_owned(),
        nullable: !line.contains("null: false"),
        primary_key: false,
        unique: line.contains("unique: true"),
    })
}

fn ruby_strings(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut quote = None;
    let mut start = 0usize;
    for (index, character) in value.char_indices() {
        match (quote, character) {
            (None, '\'' | '"') => {
                quote = Some(character);
                start = index + character.len_utf8();
            }
            (Some(open), close) if open == close => {
                result.push(value[start..index].to_owned());
                quote = None;
            }
            _ => {}
        }
    }
    result
}

fn first_ruby_string(value: &str) -> Option<String> {
    ruby_strings(value).into_iter().next()
}

fn ruby_option_string(value: &str, option: &str) -> Option<String> {
    let start = value.find(option)? + option.len();
    first_ruby_string(&value[start..])
}

fn singularize(value: &str) -> String {
    value
        .strip_suffix("ies")
        .map(|stem| format!("{stem}y"))
        .or_else(|| value.strip_suffix('s').map(ToOwned::to_owned))
        .unwrap_or_else(|| value.to_owned())
}

fn base_prisma_type(field_type: &str) -> &str {
    field_type.trim_end_matches('?').trim_end_matches("[]")
}

fn bracket_value(attributes: &str, key: &str) -> Option<String> {
    let start = attributes.find(key)? + key.len();
    let value = &attributes[start..];
    let open = value.find('[')? + 1;
    let close = value[open..].find(']')? + open;
    value[open..close]
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_sql(contents: &str) -> DatabaseSchema {
    let mut entities = Vec::new();
    let mut relationships = Vec::new();
    let upper = contents.to_ascii_uppercase();
    let mut cursor = 0;

    while let Some(offset) = upper[cursor..].find("CREATE TABLE") {
        let statement_start = cursor + offset;
        let Some(open_offset) = contents[statement_start..].find('(') else {
            break;
        };
        let open = statement_start + open_offset;
        let Some(close) = matching_parenthesis(contents, open) else {
            break;
        };
        let declaration = contents[statement_start + "CREATE TABLE".len()..open].trim();
        let name = sql_table_name(declaration);
        let body = &contents[open + 1..close];
        let mut fields = Vec::new();
        let mut table_primary_keys = Vec::new();

        for item in split_top_level(body) {
            let trimmed = item.trim();
            let upper_item = trimmed.to_ascii_uppercase();
            if upper_item.contains("FOREIGN KEY") {
                if let Some(relationship) = parse_sql_foreign_key(&name, trimmed) {
                    relationships.push(relationship);
                }
                continue;
            }
            if upper_item.starts_with("PRIMARY KEY") {
                table_primary_keys.extend(parenthesized_names(trimmed));
                continue;
            }
            if ["CONSTRAINT", "UNIQUE", "CHECK", "INDEX", "KEY"]
                .iter()
                .any(|prefix| upper_item.starts_with(prefix))
            {
                continue;
            }
            if let Some((field, inline_relationship)) = parse_sql_column(&name, trimmed) {
                fields.push(field);
                if let Some(relationship) = inline_relationship {
                    relationships.push(relationship);
                }
            }
        }
        for field in &mut fields {
            if table_primary_keys.iter().any(|key| key == &field.name) {
                field.primary_key = true;
                field.unique = true;
            }
        }
        if !name.is_empty() {
            entities.push(DatabaseEntity { name, fields });
        }
        cursor = close + 1;
    }

    DatabaseSchema {
        name: "Database schema".to_owned(),
        source_files: Vec::new(),
        entities,
        relationships,
    }
}

fn matching_parenthesis(value: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    for (offset, character) in value[open..].char_indices() {
        if matches!(character, '\'' | '"' | '`') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
        }
        if quote.is_some() {
            continue;
        }
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(value: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(&value[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(&value[start..]);
    result
}

fn sql_table_name(declaration: &str) -> String {
    let words: Vec<&str> = declaration.split_whitespace().collect();
    let candidate = if words.len() >= 4
        && words[0].eq_ignore_ascii_case("IF")
        && words[1].eq_ignore_ascii_case("NOT")
        && words[2].eq_ignore_ascii_case("EXISTS")
    {
        words[3]
    } else {
        words.first().copied().unwrap_or("")
    };
    clean_sql_identifier(candidate.rsplit('.').next().unwrap_or(candidate))
}

fn parse_sql_column(
    table: &str,
    declaration: &str,
) -> Option<(DatabaseField, Option<DatabaseRelationship>)> {
    let mut words = declaration.split_whitespace();
    let name = clean_sql_identifier(words.next()?);
    let data_type = words.next()?.trim_end_matches(',').to_owned();
    let upper = declaration.to_ascii_uppercase();
    let relationship = upper.find("REFERENCES").and_then(|index| {
        let reference = declaration[index + "REFERENCES".len()..].trim();
        let target = clean_sql_identifier(reference.split(['(', ' ']).next()?);
        let target_field = parenthesized_names(reference).into_iter().next();
        Some(DatabaseRelationship {
            from_entity: table.to_owned(),
            from_field: Some(name.clone()),
            to_entity: target,
            to_field: target_field,
            from_cardinality: Cardinality::Many,
            to_cardinality: Cardinality::One,
            label: Some(name.clone()),
        })
    });
    Some((
        DatabaseField {
            name,
            data_type,
            nullable: !upper.contains("NOT NULL"),
            primary_key: upper.contains("PRIMARY KEY"),
            unique: upper.contains("UNIQUE") || upper.contains("PRIMARY KEY"),
        },
        relationship,
    ))
}

fn parse_sql_foreign_key(table: &str, declaration: &str) -> Option<DatabaseRelationship> {
    let upper = declaration.to_ascii_uppercase();
    let foreign_index = upper.find("FOREIGN KEY")?;
    let reference_index = upper.find("REFERENCES")?;
    let from_field = parenthesized_names(&declaration[foreign_index..reference_index])
        .into_iter()
        .next();
    let reference = declaration[reference_index + "REFERENCES".len()..].trim();
    let target = clean_sql_identifier(reference.split(['(', ' ']).next()?);
    let target_field = parenthesized_names(reference).into_iter().next();
    Some(DatabaseRelationship {
        from_entity: table.to_owned(),
        from_field: from_field.clone(),
        to_entity: target,
        to_field: target_field,
        from_cardinality: Cardinality::Many,
        to_cardinality: Cardinality::One,
        label: from_field,
    })
}

fn parenthesized_names(value: &str) -> Vec<String> {
    let Some(open) = value.find('(') else {
        return Vec::new();
    };
    let Some(close) = value[open + 1..].find(')') else {
        return Vec::new();
    };
    value[open + 1..open + 1 + close]
        .split(',')
        .map(clean_sql_identifier)
        .filter(|value| !value.is_empty())
        .collect()
}

fn clean_sql_identifier(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| matches!(character, '`' | '"' | '\'' | '[' | ']'))
        .to_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum DatabaseParseError {
    #[error("could not read database definition '{}': {error}", path.display())]
    Read {
        path: PathBuf,
        error: std::io::Error,
    },
    #[error("no database entities could be parsed from the detected {0} files")]
    NoEntities(DatabaseSourceKind),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prisma_entities_and_relationships() {
        let schema = parse_prisma(
            r#"
            model User {
              id    String @id @default(cuid())
              email String @unique
              posts Post[]
            }
            model Post {
              id       String @id
              authorId String
              author   User @relation(fields: [authorId], references: [id])
            }
            "#,
        );
        assert_eq!(schema.entities.len(), 2);
        assert_eq!(schema.relationships.len(), 1);
        assert_eq!(
            schema.relationships[0].from_field.as_deref(),
            Some("authorId")
        );
        assert!(schema.entities[0].fields[0].primary_key);
    }

    #[test]
    fn parses_rails_schema_without_running_rails() {
        let schema = parse_rails_schema(
            r#"
            ActiveRecord::Schema[8.0].define(version: 2026_01_01_000001) do
              create_table "users", force: :cascade do |t|
                t.string "email", null: false
                t.index ["email"], name: "index_users_on_email", unique: true
              end

              create_table "posts", force: :cascade do |t|
                t.string "title", null: false
                t.bigint "user_id", null: false
              end

              add_foreign_key "posts", "users"
            end
            "#,
        );
        assert_eq!(schema.entities.len(), 2);
        assert_eq!(schema.relationships.len(), 1);
        assert_eq!(
            schema.relationships[0].from_field.as_deref(),
            Some("user_id")
        );
        let email = schema.entities[0]
            .fields
            .iter()
            .find(|field| field.name == "email")
            .unwrap();
        assert!(!email.nullable);
        assert!(email.unique);
    }

    #[test]
    fn parses_sql_tables_and_foreign_keys() {
        let schema = parse_sql(
            r#"
            CREATE TABLE users (id UUID PRIMARY KEY, email VARCHAR(255) NOT NULL UNIQUE);
            CREATE TABLE posts (
              id UUID PRIMARY KEY,
              user_id UUID NOT NULL,
              CONSTRAINT posts_user_fk FOREIGN KEY (user_id) REFERENCES users(id)
            );
            "#,
        );
        assert_eq!(schema.entities.len(), 2);
        assert_eq!(schema.relationships.len(), 1);
        assert_eq!(schema.relationships[0].to_entity, "users");
    }
}
