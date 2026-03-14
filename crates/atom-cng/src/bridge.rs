use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use atom_modules::{ModuleKind, ResolvedModule};
use camino::Utf8Path;

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlatbufferFieldType {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    Uint8,
    Uint16,
    Uint32,
    Uint64,
    Float32,
    Float64,
    String,
}

impl FlatbufferFieldType {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "bool" => Some(Self::Bool),
            "byte" | "int8" => Some(Self::Int8),
            "short" | "int16" => Some(Self::Int16),
            "int" | "int32" => Some(Self::Int32),
            "long" | "int64" => Some(Self::Int64),
            "ubyte" | "uint8" => Some(Self::Uint8),
            "ushort" | "uint16" => Some(Self::Uint16),
            "uint" | "uint32" => Some(Self::Uint32),
            "ulong" | "uint64" => Some(Self::Uint64),
            "float" | "float32" => Some(Self::Float32),
            "double" | "float64" => Some(Self::Float64),
            "string" => Some(Self::String),
            _ => None,
        }
    }

    fn rust_type(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int8 => "i8",
            Self::Int16 => "i16",
            Self::Int32 => "i32",
            Self::Int64 => "i64",
            Self::Uint8 => "u8",
            Self::Uint16 => "u16",
            Self::Uint32 => "u32",
            Self::Uint64 => "u64",
            Self::Float32 => "f32",
            Self::Float64 => "f64",
            Self::String => "&str",
        }
    }

    fn default_expr(&self) -> &'static str {
        match self {
            Self::Bool => "false",
            Self::Int8
            | Self::Int16
            | Self::Int32
            | Self::Int64
            | Self::Uint8
            | Self::Uint16
            | Self::Uint32
            | Self::Uint64 => "0",
            Self::Float32 | Self::Float64 => "0.0",
            Self::String => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatbufferFieldSpec {
    name: String,
    field_type: FlatbufferFieldType,
    slot_offset: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatbufferTableSpec {
    fields: Vec<FlatbufferFieldSpec>,
}

/// Render the Rust per-method exports for all Rust-backed modules in an app.
///
/// # Errors
///
/// Returns an error if a referenced schema file is missing, a request/response
/// table is absent, or a schema field uses a `FlatBuffers` type that the current
/// bridge code generator does not yet support.
pub fn render_rust_module_exports(
    repo_root: &Utf8Path,
    modules: &[ResolvedModule],
) -> AtomResult<String> {
    let mut rendered = String::new();
    for module in modules {
        if module.manifest.kind != ModuleKind::Rust || module.manifest.methods.is_empty() {
            continue;
        }

        let Some(crate_name) = module.manifest.rust_crate_name.as_deref() else {
            return Err(AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                "rust-authored modules must declare rust_crate_name",
                module.manifest.target_label.as_str(),
            ));
        };

        let tables = load_module_tables(repo_root, module)?;
        for method in &module.manifest.methods {
            let request_table = tables.get(&method.request_table).ok_or_else(|| {
                AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    format!(
                        "request_table {} is not declared by module schema files",
                        method.request_table
                    ),
                    module.manifest.target_label.as_str(),
                )
            })?;
            let response_table = tables.get(&method.response_table).ok_or_else(|| {
                AtomError::with_path(
                    AtomErrorCode::ModuleManifestInvalid,
                    format!(
                        "response_table {} is not declared by module schema files",
                        method.response_table
                    ),
                    module.manifest.target_label.as_str(),
                )
            })?;

            render_method_export(
                &mut rendered,
                crate_name,
                &module.manifest.id,
                &method.name,
                type_name(&method.request_table),
                request_table,
                type_name(&method.response_table),
                response_table,
            );
        }
    }

    Ok(rendered)
}

fn load_module_tables(
    repo_root: &Utf8Path,
    module: &ResolvedModule,
) -> AtomResult<BTreeMap<String, FlatbufferTableSpec>> {
    let mut tables = BTreeMap::new();
    for schema_file in &module.manifest.schema_files {
        let schema_path = repo_root.join(schema_file);
        let schema = fs::read_to_string(&schema_path).map_err(|error| {
            AtomError::with_path(
                AtomErrorCode::ModuleManifestInvalid,
                format!("failed to read module schema: {error}"),
                schema_path.as_str(),
            )
        })?;
        parse_schema_tables(&schema, schema_path.as_str(), &mut tables)?;
    }
    Ok(tables)
}

fn parse_schema_tables(
    schema: &str,
    schema_path: &str,
    tables: &mut BTreeMap<String, FlatbufferTableSpec>,
) -> AtomResult<()> {
    let mut namespace = String::new();
    let mut current_table_name: Option<String> = None;
    let mut current_fields = Vec::new();

    for raw_line in schema.lines() {
        let line = raw_line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("namespace ") {
            rest.trim_end_matches(';').trim().clone_into(&mut namespace);
            continue;
        }

        if current_table_name.is_none() {
            if let Some(rest) = line.strip_prefix("table ") {
                let table_decl = rest.trim();
                let table_name = table_decl
                    .trim_end_matches('{')
                    .trim_end_matches("{}")
                    .split_whitespace()
                    .next()
                    .unwrap_or_default();
                if table_name.is_empty() {
                    return Err(AtomError::with_path(
                        AtomErrorCode::ModuleManifestInvalid,
                        "table declarations must include a name",
                        schema_path,
                    ));
                }
                if table_decl.ends_with("{}") {
                    let full_name = if namespace.is_empty() {
                        table_name.to_owned()
                    } else {
                        format!("{namespace}.{table_name}")
                    };
                    tables.insert(full_name, FlatbufferTableSpec { fields: Vec::new() });
                } else {
                    current_table_name = Some(table_name.to_owned());
                    current_fields.clear();
                }
            }
            continue;
        }

        if line == "}" {
            let table_name = current_table_name.take().expect("table should exist");
            let full_name = if namespace.is_empty() {
                table_name
            } else {
                format!("{namespace}.{table_name}")
            };
            tables.insert(
                full_name,
                FlatbufferTableSpec {
                    fields: current_fields.clone(),
                },
            );
            continue;
        }

        let field_line = line.trim_end_matches(';');
        let Some((name, raw_type)) = field_line.split_once(':') else {
            continue;
        };
        let field_name = name.trim();
        let field_type = raw_type.trim().split([' ', '=']).next().unwrap_or_default();
        let Some(field_type) = FlatbufferFieldType::parse(field_type) else {
            return Err(AtomError::with_path(
                AtomErrorCode::CngTemplateError,
                format!(
                    "unsupported FlatBuffers field type '{field_type}' in {}",
                    current_table_name.as_deref().unwrap_or("table")
                ),
                schema_path,
            ));
        };
        let slot_offset = 4 + u16::try_from(current_fields.len() * 2).unwrap_or(u16::MAX);
        current_fields.push(FlatbufferFieldSpec {
            name: field_name.to_owned(),
            field_type,
            slot_offset,
        });
    }

    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "bridge codegen needs both schema and Rust type data"
)]
#[expect(
    clippy::too_many_lines,
    reason = "bridge codegen emits several helper items per method"
)]
fn render_method_export(
    rendered: &mut String,
    crate_name: &str,
    module_id: &str,
    method_name: &str,
    request_type_name: &str,
    request_table: &FlatbufferTableSpec,
    response_type_name: &str,
    response_table: &FlatbufferTableSpec,
) {
    let request_type = format!("{crate_name}::{request_type_name}");
    let response_type = format!("{crate_name}::{response_type_name}");
    let request_root_type = format!(
        "{}{}RequestFlatbuffer",
        to_camel_case(module_id),
        to_camel_case(method_name)
    );
    let decode_fn = format!("decode_{module_id}_{method_name}_request");
    let encode_fn = format!("encode_{module_id}_{method_name}_response");
    let export_fn = format!("atom_{module_id}_{method_name}");

    writeln!(
        rendered,
        r"
struct {request_root_type};

impl<'a> flatbuffers::Follow<'a> for {request_root_type} {{
    type Inner = flatbuffers::Table<'a>;

    unsafe fn follow(buf: &'a [u8], loc: usize) -> Self::Inner {{
        unsafe {{ flatbuffers::Table::new(buf, loc) }}
    }}
}}

impl flatbuffers::Verifiable for {request_root_type} {{
    fn run_verifier(
        verifier: &mut flatbuffers::Verifier,
        position: usize,
    ) -> Result<(), flatbuffers::InvalidFlatbuffer> {{
        let table = verifier.visit_table(position)?;
"
    )
    .expect("write request root");
    render_table_verifier(rendered, request_table);
    writeln!(
        rendered,
        r#"        table.finish();
        Ok(())
    }}
}}

fn {decode_fn}(input_flatbuffer: &[u8]) -> AtomResult<{request_type}> {{
    let table = flatbuffers::root::<{request_root_type}>(input_flatbuffer).map_err(|error| {{
        AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            format!("invalid FlatBuffer request for {module_id}.{method_name}: {{error}}"),
        )
    }})?;
"#
    )
    .expect("write decode header");

    if request_table.fields.is_empty() {
        writeln!(rendered, "    let _ = table;").expect("write empty request validation");
        writeln!(rendered, "    Ok({request_type} {{}})").expect("write empty request constructor");
    } else {
        for field in &request_table.fields {
            render_decode_field(rendered, field);
        }
        writeln!(rendered, "    Ok({request_type} {{").expect("write request constructor open");
        for field in &request_table.fields {
            writeln!(rendered, "        {},", field.name).expect("write request field");
        }
        writeln!(rendered, "    }})").expect("write request constructor close");
    }
    writeln!(rendered, "}}\n").expect("write decode footer");

    writeln!(
        rendered,
        r"fn {encode_fn}(response: &{response_type}) -> Vec<u8> {{
    let mut builder = flatbuffers::FlatBufferBuilder::new();
"
    )
    .expect("write encode header");
    for field in &response_table.fields {
        if field.field_type == FlatbufferFieldType::String {
            writeln!(
                rendered,
                "    let {name} = builder.create_string(&response.{name});",
                name = field.name
            )
            .expect("write string builder");
        }
    }
    writeln!(rendered, "    let table = builder.start_table();").expect("write table start");
    for field in &response_table.fields {
        render_encode_field(rendered, field);
    }
    writeln!(rendered, "    let root = builder.end_table(table);").expect("write table end");
    writeln!(rendered, "    builder.finish(root, None);").expect("write builder finish");
    writeln!(rendered, "    builder.finished_data().to_vec()").expect("write finished data");
    writeln!(rendered, "}}\n").expect("write encode footer");

    writeln!(
        rendered,
        r#"/// # Safety
///
/// `out_response_flatbuffer` must be a valid writable pointer and
/// `out_error_flatbuffer` must be null or a valid writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn {export_fn}(
    input_flatbuffer: AtomSlice,
    out_response_flatbuffer: *mut AtomOwnedBuffer,
    out_error_flatbuffer: *mut AtomOwnedBuffer,
) -> i32 {{
    if out_response_flatbuffer.is_null() {{
        let error = AtomError::new(
            AtomErrorCode::BridgeInvalidArgument,
            "{export_fn} requires a non-null out_response_flatbuffer",
        );
        unsafe {{ write_error_buffer(out_error_flatbuffer, &error) }};
        return error.exit_code();
    }}

    let handle = match active_runtime_handle() {{
        Ok(handle) => handle,
        Err(error) => {{
            unsafe {{ ptr::write(out_response_flatbuffer, AtomOwnedBuffer::empty()) }};
            unsafe {{ write_error_buffer(out_error_flatbuffer, &error) }};
            return error.exit_code();
        }}
    }};

    let request = match {decode_fn}(unsafe {{ input_flatbuffer.as_bytes() }}) {{
        Ok(request) => request,
        Err(error) => {{
            unsafe {{ ptr::write(out_response_flatbuffer, AtomOwnedBuffer::empty()) }};
            unsafe {{ write_error_buffer(out_error_flatbuffer, &error) }};
            return error.exit_code();
        }}
    }};

    match atom_runtime::call_module::<{request_type}, {response_type}>(
        handle,
        "{module_id}",
        "{method_name}",
        request,
    ) {{
        Ok(response) => {{
            let bytes = {encode_fn}(&response);
            unsafe {{
                ptr::write(out_response_flatbuffer, AtomOwnedBuffer::from_vec(bytes));
            }}
            if !out_error_flatbuffer.is_null() {{
                unsafe {{
                    ptr::write(out_error_flatbuffer, AtomOwnedBuffer::empty());
                }}
            }}
            0
        }}
        Err(error) => {{
            unsafe {{ ptr::write(out_response_flatbuffer, AtomOwnedBuffer::empty()) }};
            unsafe {{ write_error_buffer(out_error_flatbuffer, &error) }};
            error.exit_code()
        }}
    }}
}}
"#
    )
    .expect("write export");
}

fn render_decode_field(rendered: &mut String, field: &FlatbufferFieldSpec) {
    let slot = field.slot_offset;
    if field.field_type == FlatbufferFieldType::String {
        writeln!(
            rendered,
            r#"    let {name} = unsafe {{ table.get::<flatbuffers::ForwardsUOffset<&str>>({slot}, None) }}
        .ok_or_else(|| {{
            AtomError::new(
                AtomErrorCode::BridgeInvalidArgument,
                "missing required FlatBuffer field {name}",
            )
        }})?
        .to_owned();"#,
            name = field.name
        )
        .expect("write string decode");
    } else {
        let rust_type = field.field_type.rust_type();
        let default_expr = field.field_type.default_expr();
        writeln!(
            rendered,
            "    let {name} = unsafe {{ table.get::<{rust_type}>({slot}, Some({default_expr})) }}.unwrap_or({default_expr});",
            name = field.name
        )
        .expect("write scalar decode");
    }
}

fn render_encode_field(rendered: &mut String, field: &FlatbufferFieldSpec) {
    let slot = field.slot_offset;
    if field.field_type == FlatbufferFieldType::String {
        writeln!(
            rendered,
            "    builder.push_slot_always::<flatbuffers::WIPOffset<_>>({slot}, {});",
            field.name
        )
        .expect("write string encode");
    } else {
        let rust_type = field.field_type.rust_type();
        let default_expr = field.field_type.default_expr();
        writeln!(
            rendered,
            "    builder.push_slot::<{rust_type}>({slot}, response.{name}, {default_expr});",
            name = field.name
        )
        .expect("write scalar encode");
    }
}

fn type_name(qualified_table_name: &str) -> &str {
    qualified_table_name
        .rsplit('.')
        .next()
        .unwrap_or(qualified_table_name)
}

fn render_table_verifier(rendered: &mut String, table: &FlatbufferTableSpec) {
    if table.fields.is_empty() {
        writeln!(rendered, "        let table = table;").expect("write empty table verifier");
        return;
    }

    for field in &table.fields {
        match field.field_type {
            FlatbufferFieldType::String => {
                writeln!(
                    rendered,
                    "        let table = table.visit_field::<flatbuffers::ForwardsUOffset<&str>>(\"{name}\", {slot}, true)?;",
                    name = field.name,
                    slot = field.slot_offset
                )
                .expect("write string verifier");
            }
            _ => {
                writeln!(
                    rendered,
                    "        let table = table.visit_field::<{field_type}>(\"{name}\", {slot}, false)?;",
                    field_type = field.field_type.rust_type(),
                    name = field.name,
                    slot = field.slot_offset
                )
                .expect("write scalar verifier");
            }
        }
    }
}

fn to_camel_case(value: &str) -> String {
    let mut output = String::new();
    for segment in value.split('_') {
        if segment.is_empty() {
            continue;
        }
        let mut characters = segment.chars();
        if let Some(first) = characters.next() {
            output.push(first.to_ascii_uppercase());
            output.push_str(characters.as_str());
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use atom_manifest::ModuleRequest;
    use atom_modules::{MethodSpec, ModuleKind, ModuleManifest, ResolvedModule};
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    use super::{parse_schema_tables, render_rust_module_exports};

    #[test]
    fn parses_empty_and_string_field_tables() {
        let mut tables = std::collections::BTreeMap::new();
        parse_schema_tables(
            r#"
namespace atom.device_info;

table GetDeviceInfoRequest {}

table GetDeviceInfoResponse {
  model: string;
  os: string;
}
"#,
            "schema/device_info.fbs",
            &mut tables,
        )
        .expect("schema should parse");

        assert!(tables.contains_key("atom.device_info.GetDeviceInfoRequest"));
        assert_eq!(
            tables["atom.device_info.GetDeviceInfoResponse"]
                .fields
                .len(),
            2
        );
        assert_eq!(
            tables["atom.device_info.GetDeviceInfoResponse"].fields[0].slot_offset,
            4
        );
        assert_eq!(
            tables["atom.device_info.GetDeviceInfoResponse"].fields[1].slot_offset,
            6
        );
    }

    #[test]
    fn renders_rust_module_exports_for_rust_backed_methods() {
        let directory = tempdir().expect("tempdir");
        let root = Utf8PathBuf::from_path_buf(directory.path().to_path_buf()).expect("utf8");
        let schema_dir = root.join("modules/device_info/schema");
        std::fs::create_dir_all(&schema_dir).expect("schema dir");
        std::fs::write(
            schema_dir.join("device_info.fbs"),
            r#"
namespace atom.device_info;

table GetDeviceInfoRequest {}

table GetDeviceInfoResponse {
  model: string;
  os: string;
}
"#,
        )
        .expect("schema");

        let rendered = render_rust_module_exports(
            &root,
            &[ResolvedModule {
                request: ModuleRequest {
                    target_label: "//modules/device_info:device_info".to_owned(),
                },
                metadata_path: root.join("device_info.atom.module.json"),
                manifest: ModuleManifest {
                    kind: ModuleKind::Rust,
                    target_label: "//modules/device_info:device_info".to_owned(),
                    id: "device_info".to_owned(),
                    rust_crate_name: Some("device_info".to_owned()),
                    atom_api_level: 1,
                    min_atom_version: None,
                    ios_min_deployment_target: None,
                    android_min_sdk: None,
                    depends_on: Vec::new(),
                    schema_files: vec![Utf8PathBuf::from(
                        "modules/device_info/schema/device_info.fbs",
                    )],
                    methods: vec![MethodSpec {
                        name: "get".to_owned(),
                        request_table: "atom.device_info.GetDeviceInfoRequest".to_owned(),
                        response_table: "atom.device_info.GetDeviceInfoResponse".to_owned(),
                    }],
                    permissions: Vec::new(),
                    plist: serde_json::Map::new(),
                    android_manifest: serde_json::Map::new(),
                    entitlements: serde_json::Map::new(),
                    generated_sources: Vec::new(),
                    init_priority: 0,
                    ios_srcs: Vec::new(),
                    android_srcs: Vec::new(),
                },
                resolution_index: 0,
                layer: 0,
                init_order: 0,
            }],
        )
        .expect("bridge should render");

        assert!(rendered.contains("pub unsafe extern \"C\" fn atom_device_info_get("));
        assert!(rendered.contains("device_info::GetDeviceInfoRequest"));
        assert!(rendered.contains("device_info::GetDeviceInfoResponse"));
        assert!(rendered.contains("builder.create_string(&response.model)"));
        assert!(rendered.contains("builder.create_string(&response.os)"));
    }
}
