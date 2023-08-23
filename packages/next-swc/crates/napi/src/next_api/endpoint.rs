use std::ops::Deref;

use napi::{bindgen_prelude::External, JsFunction};
use next_api::route::{Endpoint, WrittenEndpoint};
use turbo_tasks::Vc;
use turbopack_binding::turbopack::core::error::PrettyPrintError;

use super::utils::{
    get_diagnostics, get_issues, subscribe, NapiDiagnostic, NapiIssue, RootTask, TurbopackResult,
    VcArc,
};

#[napi(object)]
#[derive(Default)]
pub struct NapiEndpointConfig {}

#[napi(object)]
#[derive(Default)]
pub struct NapiWrittenEndpoint {
    pub r#type: String,
    pub entry_path: Option<String>,
    pub server_paths: Option<Vec<String>>,
    pub files: Option<Vec<String>>,
    pub global_var_name: Option<String>,
    pub config: NapiEndpointConfig,
}

impl From<&WrittenEndpoint> for NapiWrittenEndpoint {
    fn from(written_endpoint: &WrittenEndpoint) -> Self {
        match written_endpoint {
            WrittenEndpoint::NodeJs {
                server_entry_path,
                server_paths,
            } => Self {
                r#type: "nodejs".to_string(),
                entry_path: Some(server_entry_path.clone()),
                server_paths: Some(server_paths.clone()),
                ..Default::default()
            },
            WrittenEndpoint::Edge {
                files,
                global_var_name,
                server_paths,
            } => Self {
                r#type: "edge".to_string(),
                files: Some(files.clone()),
                server_paths: Some(server_paths.clone()),
                global_var_name: Some(global_var_name.clone()),
                ..Default::default()
            },
        }
    }
}

// NOTE(alexkirsz) We go through an extra layer of indirection here because of
// two factors:
// 1. rustc currently has a bug where using a dyn trait as a type argument to
//    some async functions (in this case `endpoint_write_to_disk`) can cause
//    higher-ranked lifetime errors. See https://github.com/rust-lang/rust/issues/102211
// 2. the type_complexity clippy lint.
pub struct ExternalEndpoint(pub VcArc<Vc<Box<dyn Endpoint>>>);

impl Deref for ExternalEndpoint {
    type Target = VcArc<Vc<Box<dyn Endpoint>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[napi]
pub async fn endpoint_write_to_disk(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
) -> napi::Result<TurbopackResult<NapiWrittenEndpoint>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    let (written, issues, diags) = turbo_tasks
        .run_once(async move {
            let write_to_disk = endpoint.write_to_disk();
            let issues = get_issues(write_to_disk).await?;
            let diags = get_diagnostics(write_to_disk).await?;
            let written = write_to_disk.strongly_consistent().await?;
            Ok((written, issues, diags))
        })
        .await
        .map_err(|e| napi::Error::from_reason(PrettyPrintError(&e).to_string()))?;
    // TODO diagnostics
    Ok(TurbopackResult {
        result: NapiWrittenEndpoint::from(&*written),
        issues: issues.iter().map(|i| NapiIssue::from(&**i)).collect(),
        diagnostics: diags.iter().map(|d| NapiDiagnostic::from(d)).collect(),
    })
}

#[napi(ts_return_type = "{ __napiType: \"RootTask\" }")]
pub fn endpoint_server_changed_subscribe(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
    func: JsFunction,
) -> napi::Result<External<RootTask>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    subscribe(
        turbo_tasks,
        func,
        move || async move {
            let changed = endpoint.server_changed();
            let issues = get_issues(changed).await?;
            let diags = get_diagnostics(changed).await?;
            changed.strongly_consistent().await?;
            Ok((issues, diags))
        },
        |ctx| {
            let (issues, diags) = ctx.value;
            Ok(vec![TurbopackResult {
                result: (),
                issues: issues.iter().map(|i| NapiIssue::from(&**i)).collect(),
                diagnostics: diags.iter().map(|d| NapiDiagnostic::from(d)).collect(),
            }])
        },
    )
}

#[napi(ts_return_type = "{ __napiType: \"RootTask\" }")]
pub fn endpoint_client_changed_subscribe(
    #[napi(ts_arg_type = "{ __napiType: \"Endpoint\" }")] endpoint: External<ExternalEndpoint>,
    func: JsFunction,
) -> napi::Result<External<RootTask>> {
    let turbo_tasks = endpoint.turbo_tasks().clone();
    let endpoint = ***endpoint;
    subscribe(
        turbo_tasks,
        func,
        move || async move {
            let changed = endpoint.client_changed();
            let issues = get_issues(changed).await?;
            let diags = get_diagnostics(changed).await?;
            changed.strongly_consistent().await?;
            Ok((issues, diags))
        },
        |ctx| {
            let (issues, diags) = ctx.value;
            Ok(vec![TurbopackResult {
                result: (),
                issues: issues.iter().map(|i| NapiIssue::from(&**i)).collect(),
                diagnostics: diags.iter().map(|d| NapiDiagnostic::from(d)).collect(),
            }])
        },
    )
}
