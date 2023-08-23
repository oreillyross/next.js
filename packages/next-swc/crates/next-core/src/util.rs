use anyhow::{bail, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value as JsonValue;
use swc_core::ecma::ast::Program;
use turbo_tasks::{trace::TraceRawVcs, TaskInput, ValueDefault, ValueToString, Vc};
use turbo_tasks_fs::rope::Rope;
use turbopack_binding::{
    turbo::tasks_fs::{json::parse_json_rope_with_source_context, FileContent, FileSystemPath},
    turbopack::{
        core::{
            environment::{ServerAddr, ServerInfo},
            ident::AssetIdent,
            issue::{Issue, IssueExt, IssueSeverity},
            module::Module,
        },
        ecmascript::{
            analyzer::{JsValue, ObjectPart},
            parse::ParseResult,
            EcmascriptModuleAsset,
        },
        turbopack::condition::ContextCondition,
    },
};

use crate::{
    next_config::{NextConfig, OutputType},
    next_import_map::get_next_package,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, TaskInput)]
pub enum PathType {
    PagesPage,
    PagesApi,
    Data,
}

/// Converts a filename within the server root into a next pathname.
#[turbo_tasks::function]
pub async fn pathname_for_path(
    server_root: Vc<FileSystemPath>,
    server_path: Vc<FileSystemPath>,
    path_ty: PathType,
) -> Result<Vc<String>> {
    let server_path_value = &*server_path.await?;
    let path = if let Some(path) = server_root.await?.get_path_to(server_path_value) {
        path
    } else {
        bail!(
            "server_path ({}) is not in server_root ({})",
            server_path.to_string().await?,
            server_root.to_string().await?
        )
    };
    let path = match (path_ty, path) {
        // "/" is special-cased to "/index" for data routes.
        (PathType::Data, "") => "/index".to_string(),
        // `get_path_to` always strips the leading `/` from the path, so we need to add
        // it back here.
        (_, path) => format!("/{}", path),
    };

    Ok(Vc::cell(path))
}

// Adapted from https://github.com/vercel/next.js/blob/canary/packages/next/shared/lib/router/utils/get-asset-path-from-route.ts
// TODO(alexkirsz) There's no need to create an intermediate string here (and
// below), we should instead return an `impl Display`.
pub fn get_asset_prefix_from_pathname(pathname: &str) -> String {
    if pathname == "/" {
        "/index".to_string()
    } else if pathname == "/index" || pathname.starts_with("/index/") {
        format!("/index{}", pathname)
    } else {
        pathname.to_string()
    }
}

// Adapted from https://github.com/vercel/next.js/blob/canary/packages/next/shared/lib/router/utils/get-asset-path-from-route.ts
pub fn get_asset_path_from_pathname(pathname: &str, ext: &str) -> String {
    format!("{}{}", get_asset_prefix_from_pathname(pathname), ext)
}

pub async fn foreign_code_context_condition(
    next_config: Vc<NextConfig>,
) -> Result<ContextCondition> {
    let transpile_packages = next_config.transpile_packages().await?;
    let result = if transpile_packages.is_empty() {
        ContextCondition::InDirectory("node_modules".to_string())
    } else {
        ContextCondition::all(vec![
            ContextCondition::InDirectory("node_modules".to_string()),
            ContextCondition::not(ContextCondition::any(
                transpile_packages
                    .iter()
                    .map(|package| ContextCondition::InDirectory(format!("node_modules/{package}")))
                    .collect(),
            )),
        ])
    };
    Ok(result)
}

#[derive(
    Default,
    PartialEq,
    Eq,
    Clone,
    Copy,
    Debug,
    TraceRawVcs,
    Serialize,
    Deserialize,
    Hash,
    PartialOrd,
    Ord,
)]
#[serde(rename_all = "lowercase")]
pub enum NextRuntime {
    #[default]
    NodeJs,
    #[serde(alias = "experimental-edge")]
    Edge,
}

#[turbo_tasks::value]
#[derive(Default)]
pub struct NextSourceConfig {
    pub runtime: NextRuntime,

    /// Middleware router matchers
    pub matcher: Option<Vec<String>>,
}

#[turbo_tasks::value_impl]
impl ValueDefault for NextSourceConfig {
    #[turbo_tasks::function]
    pub fn value_default() -> Vc<Self> {
        NextSourceConfig::default().cell()
    }
}

/// An issue that occurred while parsing the page config.
#[turbo_tasks::value(shared)]
pub struct NextSourceConfigParsingIssue {
    ident: Vc<AssetIdent>,
    detail: Vc<String>,
}

#[turbo_tasks::value_impl]
impl Issue for NextSourceConfigParsingIssue {
    #[turbo_tasks::function]
    fn severity(&self) -> Vc<IssueSeverity> {
        IssueSeverity::Warning.into()
    }

    #[turbo_tasks::function]
    fn title(&self) -> Vc<String> {
        Vc::cell("Unable to parse config export in source file".to_string())
    }

    #[turbo_tasks::function]
    fn category(&self) -> Vc<String> {
        Vc::cell("parsing".to_string())
    }

    #[turbo_tasks::function]
    fn file_path(&self) -> Vc<FileSystemPath> {
        self.ident.path()
    }

    #[turbo_tasks::function]
    fn description(&self) -> Vc<String> {
        Vc::cell(
            "The exported configuration object in a source file need to have a very specific \
             format from which some properties can be statically parsed at compiled-time."
                .to_string(),
        )
    }

    #[turbo_tasks::function]
    fn detail(&self) -> Vc<String> {
        self.detail
    }
}

#[turbo_tasks::function]
pub async fn parse_config_from_source(module: Vc<Box<dyn Module>>) -> Result<Vc<NextSourceConfig>> {
    if let Some(ecmascript_asset) =
        Vc::try_resolve_downcast_type::<EcmascriptModuleAsset>(module).await?
    {
        if let ParseResult::Ok {
            program: Program::Module(module_ast),
            eval_context,
            ..
        } = &*ecmascript_asset.parse().await?
        {
            for item in &module_ast.body {
                if let Some(decl) = item
                    .as_module_decl()
                    .and_then(|mod_decl| mod_decl.as_export_decl())
                    .and_then(|export_decl| export_decl.decl.as_var())
                {
                    for decl in &decl.decls {
                        if decl
                            .name
                            .as_ident()
                            .map(|ident| &*ident.sym == "config")
                            .unwrap_or_default()
                        {
                            if let Some(init) = decl.init.as_ref() {
                                let value = eval_context.eval(init);
                                return Ok(parse_config_from_js_value(module, &value).cell());
                            } else {
                                NextSourceConfigParsingIssue {
                                    ident: module.ident(),
                                    detail: Vc::cell(
                                        "The exported config object must contain an variable \
                                         initializer."
                                            .to_string(),
                                    ),
                                }
                                .cell()
                                .emit()
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(Default::default())
}

fn parse_config_from_js_value(module: Vc<Box<dyn Module>>, value: &JsValue) -> NextSourceConfig {
    let mut config = NextSourceConfig::default();
    let invalid_config = |detail: &str, value: &JsValue| {
        let (explainer, hints) = value.explain(2, 0);
        NextSourceConfigParsingIssue {
            ident: module.ident(),
            detail: Vc::cell(format!("{detail} Got {explainer}.{hints}")),
        }
        .cell()
        .emit()
    };
    if let JsValue::Object { parts, .. } = value {
        for part in parts {
            match part {
                ObjectPart::Spread(_) => invalid_config(
                    "Spread properties are not supported in the config export.",
                    value,
                ),
                ObjectPart::KeyValue(key, value) => {
                    if let Some(key) = key.as_str() {
                        if key == "runtime" {
                            if let JsValue::Constant(runtime) = value {
                                if let Some(runtime) = runtime.as_str() {
                                    match runtime {
                                        "edge" | "experimental-edge" => {
                                            config.runtime = NextRuntime::Edge;
                                        }
                                        "nodejs" => {
                                            config.runtime = NextRuntime::NodeJs;
                                        }
                                        _ => {
                                            invalid_config(
                                                "The runtime property must be either \"nodejs\" \
                                                 or \"edge\".",
                                                value,
                                            );
                                        }
                                    }
                                }
                            } else {
                                invalid_config(
                                    "The runtime property must be a constant string.",
                                    value,
                                );
                            }
                        }
                        if key == "matcher" {
                            let mut matchers = vec![];
                            match value {
                                JsValue::Constant(matcher) => {
                                    if let Some(matcher) = matcher.as_str() {
                                        matchers.push(matcher.to_string());
                                    } else {
                                        invalid_config(
                                            "The matcher property must be a string or array of \
                                             strings",
                                            value,
                                        );
                                    }
                                }
                                JsValue::Array { items, .. } => {
                                    for item in items {
                                        if let Some(matcher) = item.as_str() {
                                            matchers.push(matcher.to_string());
                                        } else {
                                            invalid_config(
                                                "The matcher property must be a string or array \
                                                 of strings",
                                                value,
                                            );
                                        }
                                    }
                                }
                                _ => invalid_config(
                                    "The matcher property must be a string or array of strings",
                                    value,
                                ),
                            }
                            config.matcher = Some(matchers);
                        }
                    } else {
                        invalid_config(
                            "The exported config object must not contain non-constant strings.",
                            key,
                        );
                    }
                }
            }
        }
    } else {
        invalid_config(
            "The exported config object must be a valid object literal.",
            value,
        );
    }

    config
}

#[turbo_tasks::function]
pub async fn load_next_js_template(
    project_path: Vc<FileSystemPath>,
    path: String,
) -> Result<Vc<Rope>> {
    let file_path = get_next_package(project_path)
        .join("dist/esm".to_string())
        .join(path);

    let content = &*file_path.read().await?;

    let FileContent::Content(file) = content else {
        bail!("Expected file content for file");
    };

    Ok(file.content().to_owned().cell())
}

#[turbo_tasks::function]
pub fn virtual_next_js_template_path(
    project_path: Vc<FileSystemPath>,
    path: String,
) -> Vc<FileSystemPath> {
    get_next_package(project_path)
        .join("dist/esm".to_string())
        .join(path)
}

pub async fn load_next_js_templateon<T: DeserializeOwned>(
    project_path: Vc<FileSystemPath>,
    path: String,
) -> Result<T> {
    let file_path = get_next_package(project_path).join(path);

    let content = &*file_path.read().await?;

    let FileContent::Content(file) = content else {
        bail!("Expected file content for metrics data");
    };

    let result: T = parse_json_rope_with_source_context(file.content())?;

    Ok(result)
}

#[turbo_tasks::function]
pub async fn render_data(
    next_config: Vc<NextConfig>,
    server_addr: Vc<ServerAddr>,
) -> Result<Vc<JsonValue>> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Data {
        next_config_output: Option<OutputType>,
        server_info: Option<ServerInfo>,
        allowed_revalidate_header_keys: Option<Vec<String>>,
        fetch_cache_key_prefix: Option<String>,
        isr_memory_cache_size: Option<f64>,
        isr_flush_to_disk: Option<bool>,
    }

    let config = next_config.await?;
    let server_info = ServerInfo::try_from(&*server_addr.await?);

    let experimental = &config.experimental;

    let value = serde_json::to_value(Data {
        next_config_output: config.output.clone(),
        server_info: server_info.ok(),
        allowed_revalidate_header_keys: experimental.allowed_revalidate_header_keys.clone(),
        fetch_cache_key_prefix: experimental.fetch_cache_key_prefix.clone(),
        isr_memory_cache_size: experimental.isr_memory_cache_size,
        isr_flush_to_disk: experimental.isr_flush_to_disk,
    })?;
    Ok(Vc::cell(value))
}
