use std::collections::BTreeSet;
use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};
use serde_json::json;
use syn::{
    Attribute, Fields, ForeignItem, Item, ItemEnum, ItemFn, ItemForeignMod, ItemMod, ItemStruct,
    ReturnType, Type,
};

use crate::templates;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRustModule {
    pub records: Vec<Record>,
    pub exports: Vec<FunctionSignature>,
    pub imports: Vec<FunctionSignature>,
}

impl ParsedRustModule {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.exports.is_empty() && self.imports.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Struct(StructRecord),
    SimpleEnum(SimpleEnumRecord),
    DataEnum(DataEnumRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructRecord {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleEnumRecord {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataEnumRecord {
    pub name: String,
    pub variants: Vec<DataEnumVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataEnumVariant {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSignature {
    pub name: String,
    pub params: Vec<Field>,
    pub return_type: Option<RustType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: RustType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustType {
    Primitive(&'static str),
    String,
    Vec(Box<RustType>),
    Option(Box<RustType>),
    Result {
        ok: Box<RustType>,
        err: Box<RustType>,
    },
    Record(String),
    Unit,
}

#[derive(Debug, Default)]
struct ParseState {
    records: Vec<Record>,
    exports: Vec<FunctionSignature>,
    imports: Vec<FunctionSignature>,
    visited: BTreeSet<Utf8PathBuf>,
}

/// # Errors
///
/// Returns an error if the module source tree cannot be read, parsed, or mapped to the supported
/// `FlatBuffers` schema subset.
pub fn parse_rust_module(
    repo_root: &Utf8Path,
    crate_root: &Utf8Path,
) -> AtomResult<ParsedRustModule> {
    let crate_root = repo_root.join(crate_root);
    let mut state = ParseState::default();
    visit_file(&crate_root, &mut state)?;
    Ok(ParsedRustModule {
        records: state.records,
        exports: state.exports,
        imports: state.imports,
    })
}

/// # Errors
///
/// Returns an error if the parsed Rust items cannot be rendered to a valid `FlatBuffers` schema
/// string.
pub fn render_flatbuffer_schema(module_id: &str, parsed: &ParsedRustModule) -> AtomResult<String> {
    let namespace = format!("atom.{}", module_id.replace('-', "_"));
    let mut blocks = Vec::new();
    for record in &parsed.records {
        match record {
            Record::Struct(record) => {
                blocks.push(table_block(&record.name, &record.fields)?);
            }
            Record::SimpleEnum(record) => {
                blocks.push(json!({
                    "kind": "enum",
                    "name": record.name,
                    "variants": record.variants,
                }));
            }
            Record::DataEnum(record) => {
                for variant in &record.variants {
                    blocks.push(table_block(
                        &format!("{}{}", record.name, variant.name),
                        &variant.fields,
                    )?);
                }
                blocks.push(json!({
                    "kind": "union",
                    "name": format!("{}Union", record.name),
                    "variants": record
                        .variants
                        .iter()
                        .map(|variant| format!("{}{}", record.name, variant.name))
                        .collect::<Vec<_>>(),
                }));
                blocks.push(table_block(
                    &record.name,
                    &[Field {
                        name: "value".to_owned(),
                        ty: RustType::Record(format!("{}Union", record.name)),
                    }],
                )?);
            }
        }
    }
    for import in &parsed.imports {
        blocks.push(table_block(
            &format!("{}Args", upper_camel_case(&import.name)),
            &import.params,
        )?);
    }
    templates::render(
        "flatbuffers/module.fbs",
        minijinja::context! {
            blocks,
            namespace,
        },
    )
}

fn table_block(name: &str, fields: &[Field]) -> AtomResult<serde_json::Value> {
    Ok(json!({
        "fields": fields
            .iter()
            .map(|field| {
                Ok(json!({
                    "name": field.name,
                    "ty": render_type(&field.ty)?,
                }))
            })
            .collect::<AtomResult<Vec<_>>>()?,
        "kind": "table",
        "name": name,
    }))
}

fn visit_file(path: &Utf8Path, state: &mut ParseState) -> AtomResult<()> {
    if !state.visited.insert(path.to_owned()) {
        return Ok(());
    }

    let source = fs::read_to_string(path).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngTemplateError,
            format!("failed to read Rust source: {error}"),
            path.as_str(),
        )
    })?;
    let file = syn::parse_file(&source).map_err(|error| {
        AtomError::with_path(
            AtomErrorCode::CngTemplateError,
            format!("failed to parse Rust source: {error}"),
            path.as_str(),
        )
    })?;
    let module_dir = module_directory(path);
    for item in file.items {
        visit_item(item, &module_dir, path, state)?;
    }
    Ok(())
}

fn visit_item(
    item: Item,
    module_dir: &Utf8Path,
    current_file: &Utf8Path,
    state: &mut ParseState,
) -> AtomResult<()> {
    match item {
        Item::Struct(item) if has_attr(&item.attrs, "atom_record") => {
            state.records.push(Record::Struct(parse_struct(item)?));
        }
        Item::Enum(item) if has_attr(&item.attrs, "atom_record") => {
            state.records.push(parse_enum(item)?);
        }
        Item::Fn(item) if has_attr(&item.attrs, "atom_export") => {
            state
                .exports
                .push(parse_function(item.sig.ident.to_string(), &item)?);
        }
        Item::ForeignMod(item) if has_attr(&item.attrs, "atom_import") => {
            parse_import_block(item, state)?;
        }
        Item::Mod(item) => {
            visit_module(item, module_dir, current_file, state)?;
        }
        _ => {}
    }
    Ok(())
}

fn visit_module(
    item: ItemMod,
    module_dir: &Utf8Path,
    current_file: &Utf8Path,
    state: &mut ParseState,
) -> AtomResult<()> {
    if let Some((_, items)) = item.content {
        let next_module_dir = module_dir.join(item.ident.to_string());
        for nested in items {
            visit_item(nested, &next_module_dir, current_file, state)?;
        }
        return Ok(());
    }

    let path = resolve_module_path(module_dir, current_file, &item)?;
    visit_file(&path, state)
}

fn parse_import_block(item: ItemForeignMod, state: &mut ParseState) -> AtomResult<()> {
    for foreign_item in item.items {
        let ForeignItem::Fn(function) = foreign_item else {
            continue;
        };
        state.imports.push(parse_foreign_function(
            function.sig.ident.to_string(),
            &function.sig,
        )?);
    }
    Ok(())
}

fn parse_struct(item: ItemStruct) -> AtomResult<StructRecord> {
    Ok(StructRecord {
        name: item.ident.to_string(),
        fields: parse_fields(item.fields)?,
    })
}

fn parse_enum(item: ItemEnum) -> AtomResult<Record> {
    let variants: Vec<DataEnumVariant> = item
        .variants
        .into_iter()
        .map(|variant| {
            Ok(DataEnumVariant {
                name: variant.ident.to_string(),
                fields: parse_fields(variant.fields)?,
            })
        })
        .collect::<AtomResult<_>>()?;
    if variants.iter().all(|variant| variant.fields.is_empty()) {
        return Ok(Record::SimpleEnum(SimpleEnumRecord {
            name: item.ident.to_string(),
            variants: variants.into_iter().map(|variant| variant.name).collect(),
        }));
    }
    Ok(Record::DataEnum(DataEnumRecord {
        name: item.ident.to_string(),
        variants,
    }))
}

fn parse_function(name: String, item: &ItemFn) -> AtomResult<FunctionSignature> {
    Ok(FunctionSignature {
        name,
        params: item
            .sig
            .inputs
            .iter()
            .map(parse_fn_arg)
            .collect::<AtomResult<_>>()?,
        return_type: parse_return_type(&item.sig.output)?,
    })
}

fn parse_foreign_function(
    name: String,
    signature: &syn::Signature,
) -> AtomResult<FunctionSignature> {
    Ok(FunctionSignature {
        name,
        params: signature
            .inputs
            .iter()
            .map(parse_fn_arg)
            .collect::<AtomResult<_>>()?,
        return_type: parse_return_type(&signature.output)?,
    })
}

fn parse_fn_arg(arg: &syn::FnArg) -> AtomResult<Field> {
    match arg {
        syn::FnArg::Receiver(_) => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "atom schema parsing does not support self receivers",
        )),
        syn::FnArg::Typed(arg) => {
            let name = match arg.pat.as_ref() {
                syn::Pat::Ident(ident) => ident.ident.to_string(),
                _ => "value".to_owned(),
            };
            Ok(Field {
                name,
                ty: parse_type(&arg.ty)?,
            })
        }
    }
}

fn parse_return_type(output: &ReturnType) -> AtomResult<Option<RustType>> {
    match output {
        ReturnType::Default => Ok(None),
        ReturnType::Type(_, ty) => {
            let parsed = parse_type(ty)?;
            Ok((parsed != RustType::Unit).then_some(parsed))
        }
    }
}

fn parse_fields(fields: Fields) -> AtomResult<Vec<Field>> {
    match fields {
        Fields::Named(fields) => fields
            .named
            .into_iter()
            .map(|field| {
                Ok(Field {
                    name: field
                        .ident
                        .map_or_else(|| "value".to_owned(), |ident| ident.to_string()),
                    ty: parse_type(&field.ty)?,
                })
            })
            .collect(),
        Fields::Unnamed(fields) => fields
            .unnamed
            .into_iter()
            .enumerate()
            .map(|(index, field)| {
                Ok(Field {
                    name: format!("value{index}"),
                    ty: parse_type(&field.ty)?,
                })
            })
            .collect(),
        Fields::Unit => Ok(Vec::new()),
    }
}

fn parse_type(ty: &Type) -> AtomResult<RustType> {
    match ty {
        Type::Path(path) => parse_path_type(&path.path),
        Type::Reference(reference) => {
            let inner = parse_type(&reference.elem)?;
            Ok(match inner {
                RustType::Record(name) => RustType::Record(name),
                RustType::Primitive("str") | RustType::String => RustType::String,
                other => other,
            })
        }
        Type::Tuple(tuple) if tuple.elems.is_empty() => Ok(RustType::Unit),
        _ => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "unsupported Rust type in atom schema metadata",
        )),
    }
}

fn parse_path_type(path: &syn::Path) -> AtomResult<RustType> {
    let Some(segment) = path.segments.last() else {
        return Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "unsupported Rust type in atom schema metadata",
        ));
    };
    let ident = segment.ident.to_string();
    match ident.as_str() {
        "String" => Ok(RustType::String),
        "str" => Ok(RustType::Primitive("str")),
        "Vec" => Ok(RustType::Vec(Box::new(parse_single_generic(segment)?))),
        "Option" => Ok(RustType::Option(Box::new(parse_single_generic(segment)?))),
        "Result" => {
            let mut generics = parse_generic_types(segment)?;
            if generics.len() != 2 {
                return Err(AtomError::new(
                    AtomErrorCode::CngTemplateError,
                    "Result in atom schema metadata must have exactly two type arguments",
                ));
            }
            let err = Box::new(generics.pop().expect("err type"));
            let ok = Box::new(generics.pop().expect("ok type"));
            Ok(RustType::Result { ok, err })
        }
        "i8" => Ok(RustType::Primitive("i8")),
        "i16" => Ok(RustType::Primitive("i16")),
        "i32" => Ok(RustType::Primitive("i32")),
        "i64" => Ok(RustType::Primitive("i64")),
        "u8" => Ok(RustType::Primitive("u8")),
        "u16" => Ok(RustType::Primitive("u16")),
        "u32" => Ok(RustType::Primitive("u32")),
        "u64" => Ok(RustType::Primitive("u64")),
        "f32" => Ok(RustType::Primitive("f32")),
        "f64" => Ok(RustType::Primitive("f64")),
        "bool" => Ok(RustType::Primitive("bool")),
        _ => Ok(RustType::Record(ident)),
    }
}

fn parse_single_generic(segment: &syn::PathSegment) -> AtomResult<RustType> {
    let mut generics = parse_generic_types(segment)?;
    if generics.len() != 1 {
        return Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "atom schema metadata expects a single generic argument",
        ));
    }
    Ok(generics.pop().expect("single generic"))
}

fn parse_generic_types(segment: &syn::PathSegment) -> AtomResult<Vec<RustType>> {
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "atom schema metadata expects angle-bracketed type arguments",
        ));
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(parse_type(ty)),
            _ => None,
        })
        .collect()
}

fn render_type(ty: &RustType) -> AtomResult<String> {
    match ty {
        RustType::Primitive("i8") => Ok("byte".to_owned()),
        RustType::Primitive("i16") => Ok("short".to_owned()),
        RustType::Primitive("i32") => Ok("int".to_owned()),
        RustType::Primitive("i64") => Ok("long".to_owned()),
        RustType::Primitive("u8") => Ok("ubyte".to_owned()),
        RustType::Primitive("u16") => Ok("ushort".to_owned()),
        RustType::Primitive("u32") => Ok("uint".to_owned()),
        RustType::Primitive("u64") => Ok("ulong".to_owned()),
        RustType::Primitive("f32") => Ok("float".to_owned()),
        RustType::Primitive("f64") => Ok("double".to_owned()),
        RustType::Primitive("bool") => Ok("bool".to_owned()),
        RustType::Primitive("str") | RustType::String => Ok("string".to_owned()),
        RustType::Vec(inner) => Ok(format!("[{}]", render_type(inner)?)),
        RustType::Option(inner) => render_type(inner),
        RustType::Result { .. } => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "Result types are not valid FlatBuffers field types",
        )),
        RustType::Record(name) => Ok(name.clone()),
        RustType::Unit => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "unit types are not valid FlatBuffers field types",
        )),
        RustType::Primitive(_) => Err(AtomError::new(
            AtomErrorCode::CngTemplateError,
            "unsupported Rust primitive in atom schema metadata",
        )),
    }
}

fn resolve_module_path(
    module_dir: &Utf8Path,
    current_file: &Utf8Path,
    item: &ItemMod,
) -> AtomResult<Utf8PathBuf> {
    if let Some(path_attr) = attr_path_value(&item.attrs) {
        let candidate = module_dir.join(path_attr);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    let stem = item.ident.to_string();
    let candidates = [
        module_dir.join(format!("{stem}.rs")),
        module_dir.join(stem).join("mod.rs"),
    ];
    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(AtomError::with_path(
        AtomErrorCode::CngTemplateError,
        format!("failed to resolve Rust module file for `{}`", item.ident),
        current_file.as_str(),
    ))
}

fn module_directory(path: &Utf8Path) -> Utf8PathBuf {
    let parent = path.parent().unwrap_or_else(|| Utf8Path::new(""));
    let file_name = path.file_name().unwrap_or_default();
    if matches!(file_name, "lib.rs" | "mod.rs") {
        return parent.to_owned();
    }

    let stem = path.file_stem().unwrap_or_default();
    if stem.is_empty() {
        return parent.to_owned();
    }
    parent.join(stem)
}

fn has_attr(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == name)
    })
}

fn attr_path_value(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(meta) = &attr.meta else {
            return None;
        };
        let syn::Expr::Lit(expr) = &meta.value else {
            return None;
        };
        let syn::Lit::Str(value) = &expr.lit else {
            return None;
        };
        Some(value.value())
    })
}

fn upper_camel_case(value: &str) -> String {
    value
        .split('_')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let mut output = String::new();
            if let Some(first) = chars.next() {
                output.push(first.to_ascii_uppercase());
            }
            output.extend(chars);
            output
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{Record, parse_rust_module, render_flatbuffer_schema};

    #[test]
    fn parses_rust_module_records_exports_and_imports() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("module/src")).expect("src dir");
        fs::write(
            root.join("module/src/lib.rs"),
            r#"
#[atom_macros::atom_record]
struct EchoRequest {
    message: String,
}

#[atom_macros::atom_record]
enum ConnectionStatus {
    Connected,
    Disconnected,
}

#[atom_macros::atom_record]
enum Payload {
    Message { value: String },
    Count(i32),
}

#[atom_macros::atom_export]
fn echo(request: EchoRequest) -> Result<String, AtomError> {
    todo!()
}

#[atom_macros::atom_import]
extern "C" {
    fn set(key: String, value: String);
}
"#,
        )
        .expect("source");

        let parsed = parse_rust_module(&root, Utf8PathBuf::from("module/src/lib.rs").as_path())
            .expect("parse");

        assert_eq!(parsed.records.len(), 3);
        assert!(matches!(parsed.records[0], Record::Struct(_)));
        assert_eq!(parsed.exports.len(), 1);
        assert_eq!(parsed.exports[0].name, "echo");
        assert_eq!(parsed.imports.len(), 1);
        assert_eq!(parsed.imports[0].name, "set");
    }

    #[test]
    fn resolves_nested_modules_and_renders_schema() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8 path");
        fs::create_dir_all(root.join("module/src/nested")).expect("nested dir");
        fs::write(
            root.join("module/src/lib.rs"),
            r#"
mod nested;

#[atom_macros::atom_import]
extern "C" {
    fn set(key: String, value: String);
}
"#,
        )
        .expect("root source");
        fs::write(
            root.join("module/src/nested.rs"),
            r#"
#[atom_macros::atom_record]
struct DeviceInfo {
    model: String,
    os: String,
    status: ConnectionStatus,
    payloads: Vec<Payload>,
}

#[atom_macros::atom_record]
enum ConnectionStatus {
    Connected,
    Disconnected,
}

#[atom_macros::atom_record]
enum Payload {
    Message { value: String },
    Count(i32),
}
"#,
        )
        .expect("nested source");

        let parsed = parse_rust_module(&root, Utf8PathBuf::from("module/src/lib.rs").as_path())
            .expect("parse");
        let rendered = render_flatbuffer_schema("device_info", &parsed).expect("render");

        assert!(rendered.contains("namespace atom.device_info;"));
        assert!(rendered.contains("table DeviceInfo"));
        assert!(rendered.contains("enum ConnectionStatus: ubyte"));
        assert!(rendered.contains("union PayloadUnion"));
        assert!(rendered.contains("table SetArgs"));
        assert!(rendered.contains("payloads: [Payload];"));
    }
}
