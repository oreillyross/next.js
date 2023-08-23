use std::{collections::HashMap, future::Future, ops::Deref, sync::Arc};

use anyhow::{anyhow, Context, Result};
use napi::{
    bindgen_prelude::{External, ToNapiValue},
    threadsafe_function::{ThreadSafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode},
    JsFunction, JsObject, JsUnknown, NapiRaw, NapiValue, Status,
};
use serde::Serialize;
use turbo_tasks::{unit, ReadRef, TaskId, TryJoinIterExt, TurboTasks, Vc};
use turbopack_binding::{
    turbo::{tasks_fs::FileContent, tasks_memory::MemoryBackend},
    turbopack::core::{
        diagnostics::{Diagnostic, DiagnosticContextExt, PlainDiagnostic},
        error::PrettyPrintError,
        issue::{IssueDescriptionExt, PlainIssue, PlainIssueSource, PlainSource},
        source_pos::SourcePos,
    },
};

/// A helper type to hold both a Vc operation and the TurboTasks root process.
/// Without this, we'd need to pass both individually all over the place
#[derive(Clone)]
pub struct VcArc<T> {
    turbo_tasks: Arc<TurboTasks<MemoryBackend>>,
    /// The Vc. Must be resolved, otherwise you are referencing an inactive
    /// operation.
    vc: T,
}

impl<T> VcArc<T> {
    pub fn new(turbo_tasks: Arc<TurboTasks<MemoryBackend>>, vc: T) -> Self {
        Self { turbo_tasks, vc }
    }

    pub fn turbo_tasks(&self) -> &Arc<TurboTasks<MemoryBackend>> {
        &self.turbo_tasks
    }
}

impl<T> Deref for VcArc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.vc
    }
}

pub fn serde_enum_to_string<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_value(value)?
        .as_str()
        .context("value must serialize to a string")?
        .to_string())
}

/// The root of our turbopack computation.
pub struct RootTask {
    #[allow(dead_code)]
    turbo_tasks: Arc<TurboTasks<MemoryBackend>>,
    #[allow(dead_code)]
    task_id: Option<TaskId>,
}

impl Drop for RootTask {
    fn drop(&mut self) {
        // TODO stop the root task
    }
}

#[napi]
pub fn root_task_dispose(
    #[napi(ts_arg_type = "{ __napiType: \"RootTask\" }")] _root_task: External<RootTask>,
) -> napi::Result<()> {
    // TODO(alexkirsz) Implement. Not panicking here to avoid crashing the process
    // when testing.
    eprintln!("root_task_dispose not yet implemented");
    Ok(())
}

pub async fn get_issues<T>(source: Vc<T>) -> Result<Vec<ReadRef<PlainIssue>>> {
    let issues = source
        .peek_issues_with_path()
        .await?
        .strongly_consistent()
        .await?;
    issues.get_plain_issues().await
}

/// Collect [turbopack::core::diagnostics::Diagnostic] from given source,
/// returns [turbopack::core::diagnostics::PlainDiagnostic]
pub async fn get_diagnostics<T>(source: Vc<T>) -> Result<Vec<ReadRef<PlainDiagnostic>>> {
    let captured_diags = source
        .peek_diagnostics()
        .await?
        .strongly_consistent()
        .await?;

    captured_diags
        .diagnostics
        .iter()
        .map(|d| d.into_plain())
        .try_join()
        .await
}

#[napi(object)]
pub struct NapiIssue {
    pub severity: String,
    pub category: String,
    pub file_path: String,
    pub title: String,
    pub description: String,
    pub detail: String,
    pub source: Option<NapiIssueSource>,
    pub documentation_link: String,
    pub sub_issues: Vec<NapiIssue>,
}

impl From<&PlainIssue> for NapiIssue {
    fn from(issue: &PlainIssue) -> Self {
        Self {
            description: issue.description.clone(),
            category: issue.category.clone(),
            file_path: issue.file_path.clone(),
            detail: issue.detail.clone(),
            documentation_link: issue.documentation_link.clone(),
            severity: issue.severity.as_str().to_string(),
            source: issue.source.as_deref().map(|source| source.into()),
            title: issue.title.clone(),
            sub_issues: issue
                .sub_issues
                .iter()
                .map(|issue| (&**issue).into())
                .collect(),
        }
    }
}

#[napi(object)]
pub struct NapiIssueSource {
    pub source: NapiSource,
    pub start: NapiSourcePos,
    pub end: NapiSourcePos,
}

impl From<&PlainIssueSource> for NapiIssueSource {
    fn from(
        PlainIssueSource {
            asset: source,
            start,
            end,
        }: &PlainIssueSource,
    ) -> Self {
        Self {
            source: (&**source).into(),
            start: (*start).into(),
            end: (*end).into(),
        }
    }
}

#[napi(object)]
pub struct NapiSource {
    pub ident: String,
    pub content: Option<String>,
}

impl From<&PlainSource> for NapiSource {
    fn from(source: &PlainSource) -> Self {
        Self {
            ident: source.ident.to_string(),
            content: match &*source.content {
                FileContent::Content(content) => match content.content().to_str() {
                    Ok(str) => Some(str.into_owned()),
                    Err(_) => None,
                },
                FileContent::NotFound => None,
            },
        }
    }
}

#[napi(object)]
pub struct NapiSourcePos {
    pub line: u32,
    pub column: u32,
}

impl From<SourcePos> for NapiSourcePos {
    fn from(pos: SourcePos) -> Self {
        Self {
            line: pos.line as u32,
            column: pos.column as u32,
        }
    }
}

#[napi(object)]
pub struct NapiDiagnostic {
    pub category: String,
    pub name: String,
    pub payload: HashMap<String, String>,
}

impl NapiDiagnostic {
    pub fn from(diagnostic: &PlainDiagnostic) -> Self {
        Self {
            category: diagnostic.category.clone(),
            name: diagnostic.name.clone(),
            payload: diagnostic.payload.clone(),
        }
    }
}

pub struct TurbopackResult<T: ToNapiValue> {
    pub result: T,
    pub issues: Vec<NapiIssue>,
    pub diagnostics: Vec<NapiDiagnostic>,
}

impl<T: ToNapiValue> ToNapiValue for TurbopackResult<T> {
    unsafe fn to_napi_value(
        env: napi::sys::napi_env,
        val: Self,
    ) -> napi::Result<napi::sys::napi_value> {
        let mut obj = napi::Env::from_raw(env).create_object()?;

        let result = T::to_napi_value(env, val.result)?;
        let result = JsUnknown::from_raw(env, result)?;
        if matches!(result.get_type()?, napi::ValueType::Object) {
            // SAFETY: We know that result is an object, so we can cast it to a JsObject
            let result = unsafe { result.cast::<JsObject>() };

            for key in JsObject::keys(&result)? {
                let value: JsUnknown = result.get_named_property(&key)?;
                obj.set_named_property(&key, value)?;
            }
        }

        obj.set_named_property("issues", val.issues)?;
        obj.set_named_property("diagnostics", val.diagnostics)?;

        Ok(obj.raw())
    }
}

pub fn subscribe<T: 'static + Send + Sync, F: Future<Output = Result<T>> + Send, V: ToNapiValue>(
    turbo_tasks: Arc<TurboTasks<MemoryBackend>>,
    func: JsFunction,
    handler: impl 'static + Sync + Send + Clone + Fn() -> F,
    mapper: impl 'static + Sync + Send + FnMut(ThreadSafeCallContext<T>) -> napi::Result<Vec<V>>,
) -> napi::Result<External<RootTask>> {
    let func: ThreadsafeFunction<T> = func.create_threadsafe_function(0, mapper)?;
    let task_id = turbo_tasks.spawn_root_task(move || {
        let handler = handler.clone();
        let func = func.clone();
        Box::pin(async move {
            let result = handler().await;

            let status = func.call(
                result.map_err(|e| napi::Error::from_reason(PrettyPrintError(&e).to_string())),
                ThreadsafeFunctionCallMode::NonBlocking,
            );
            if !matches!(status, Status::Ok) {
                let error = anyhow!("Error calling JS function: {}", status);
                eprintln!("{}", error);
                return Err(error);
            }
            Ok(unit())
        })
    });
    Ok(External::new(RootTask {
        turbo_tasks,
        task_id: Some(task_id),
    }))
}
