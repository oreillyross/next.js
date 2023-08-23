use std::collections::{BTreeMap, HashMap};

use anyhow::{bail, Result};
use indexmap::{indexmap, map::Entry, IndexMap};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use turbo_tasks::{
    debug::ValueDebugFormat, trace::TraceRawVcs, Completion, Completions, TaskInput, ValueToString,
    Vc,
};
use turbopack_binding::{
    turbo::tasks_fs::{DirectoryContent, DirectoryEntry, FileSystemEntryType, FileSystemPath},
    turbopack::core::issue::{Issue, IssueExt, IssueSeverity},
};

use crate::{next_config::NextConfig, next_import_map::get_next_package};

/// A final route in the app directory.
#[turbo_tasks::value]
#[derive(Default, Debug, Clone)]
pub struct Components {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loading: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_found: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<Vc<FileSystemPath>>,
    #[serde(skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

impl Components {
    fn without_leafs(&self) -> Self {
        Self {
            page: None,
            layout: self.layout,
            error: self.error,
            loading: self.loading,
            template: self.template,
            not_found: self.not_found,
            default: None,
            route: None,
            metadata: self.metadata.clone(),
        }
    }

    fn merge(a: &Self, b: &Self) -> Self {
        Self {
            page: a.page.or(b.page),
            layout: a.layout.or(b.layout),
            error: a.error.or(b.error),
            loading: a.loading.or(b.loading),
            template: a.template.or(b.template),
            not_found: a.not_found.or(b.not_found),
            default: a.default.or(b.default),
            route: a.route.or(b.route),
            metadata: Metadata::merge(&a.metadata, &b.metadata),
        }
    }
}

#[turbo_tasks::value_impl]
impl Components {
    /// Returns a completion that changes when any route in the components
    /// changes.
    #[turbo_tasks::function]
    pub async fn routes_changed(self: Vc<Self>) -> Result<Vc<Completion>> {
        self.await?;
        Ok(Completion::new())
    }
}

/// A single metadata file plus an optional "alt" text file.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TraceRawVcs)]
pub enum MetadataWithAltItem {
    Static {
        path: Vc<FileSystemPath>,
        alt_path: Option<Vc<FileSystemPath>>,
    },
    Dynamic {
        path: Vc<FileSystemPath>,
    },
}

/// A single metadata file.
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TaskInput, TraceRawVcs)]
pub enum MetadataItem {
    Static { path: Vc<FileSystemPath> },
    Dynamic { path: Vc<FileSystemPath> },
}

/// Metadata file that can be placed in any segment of the app directory.
#[derive(Default, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TraceRawVcs)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub icon: Vec<MetadataWithAltItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub apple: Vec<MetadataWithAltItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub twitter: Vec<MetadataWithAltItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub open_graph: Vec<MetadataWithAltItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub favicon: Vec<MetadataWithAltItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<MetadataItem>,
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        let Metadata {
            icon,
            apple,
            twitter,
            open_graph,
            favicon,
            manifest,
        } = self;
        icon.is_empty()
            && apple.is_empty()
            && twitter.is_empty()
            && open_graph.is_empty()
            && favicon.is_empty()
            && manifest.is_none()
    }

    fn merge(a: &Self, b: &Self) -> Self {
        Self {
            icon: a.icon.iter().chain(b.icon.iter()).copied().collect(),
            apple: a.apple.iter().chain(b.apple.iter()).copied().collect(),
            twitter: a.twitter.iter().chain(b.twitter.iter()).copied().collect(),
            open_graph: a
                .open_graph
                .iter()
                .chain(b.open_graph.iter())
                .copied()
                .collect(),
            favicon: a.favicon.iter().chain(b.favicon.iter()).copied().collect(),
            manifest: a.manifest.or(b.manifest),
        }
    }
}

/// Metadata files that can be placed in the root of the app directory.
#[turbo_tasks::value]
#[derive(Default, Clone, Debug)]
pub struct GlobalMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<MetadataItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub robots: Option<MetadataItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sitemap: Option<MetadataItem>,
}

impl GlobalMetadata {
    pub fn is_empty(&self) -> bool {
        let GlobalMetadata {
            favicon,
            robots,
            sitemap,
        } = self;
        favicon.is_none() && robots.is_none() && sitemap.is_none()
    }
}

#[turbo_tasks::value]
#[derive(Debug)]
pub struct DirectoryTree {
    /// key is e.g. "dashboard", "(dashboard)", "@slot"
    pub subdirectories: BTreeMap<String, Vc<DirectoryTree>>,
    pub components: Vc<Components>,
}

#[turbo_tasks::value_impl]
impl DirectoryTree {
    /// Returns a completion that changes when any route in the whole tree
    /// changes.
    #[turbo_tasks::function]
    pub async fn routes_changed(self: Vc<Self>) -> Result<Vc<Completion>> {
        let DirectoryTree {
            subdirectories,
            components,
        } = &*self.await?;
        let mut children = Vec::new();
        children.push(components.routes_changed());
        for child in subdirectories.values() {
            children.push(child.routes_changed());
        }
        Ok(Vc::<Completions>::cell(children).completed())
    }
}

#[turbo_tasks::value(transparent)]
pub struct OptionAppDir(Option<Vc<FileSystemPath>>);

#[turbo_tasks::value_impl]
impl OptionAppDir {
    /// Returns a completion that changes when any route in the whole tree
    /// changes.
    #[turbo_tasks::function]
    pub async fn routes_changed(
        self: Vc<Self>,
        next_config: Vc<NextConfig>,
    ) -> Result<Vc<Completion>> {
        if let Some(app_dir) = *self.await? {
            let directory_tree = get_directory_tree(app_dir, next_config.page_extensions());
            directory_tree.routes_changed().await?;
        }
        Ok(Completion::new())
    }
}

/// Finds and returns the [DirectoryTree] of the app directory if existing.
#[turbo_tasks::function]
pub async fn find_app_dir(project_path: Vc<FileSystemPath>) -> Result<Vc<OptionAppDir>> {
    let app = project_path.join("app".to_string());
    let src_app = project_path.join("src/app".to_string());
    let app_dir = if *app.get_type().await? == FileSystemEntryType::Directory {
        app
    } else if *src_app.get_type().await? == FileSystemEntryType::Directory {
        src_app
    } else {
        return Ok(Vc::cell(None));
    }
    .resolve()
    .await?;

    Ok(Vc::cell(Some(app_dir)))
}

/// Finds and returns the [DirectoryTree] of the app directory if enabled and
/// existing.
#[turbo_tasks::function]
pub async fn find_app_dir_if_enabled(
    project_path: Vc<FileSystemPath>,
    next_config: Vc<NextConfig>,
) -> Result<Vc<OptionAppDir>> {
    if !*next_config.app_dir().await? {
        return Ok(Vc::cell(None));
    }
    Ok(find_app_dir(project_path))
}

static STATIC_LOCAL_METADATA: Lazy<HashMap<&'static str, &'static [&'static str]>> =
    Lazy::new(|| {
        HashMap::from([
            (
                "icon",
                &["ico", "jpg", "jpeg", "png", "svg"] as &'static [&'static str],
            ),
            ("apple-icon", &["jpg", "jpeg", "png"]),
            ("opengraph-image", &["jpg", "jpeg", "png", "gif"]),
            ("twitter-image", &["jpg", "jpeg", "png", "gif"]),
            ("favicon", &["ico"]),
            ("manifest", &["webmanifest", "json"]),
        ])
    });

static STATIC_GLOBAL_METADATA: Lazy<HashMap<&'static str, &'static [&'static str]>> =
    Lazy::new(|| {
        HashMap::from([
            ("favicon", &["ico"] as &'static [&'static str]),
            ("robots", &["txt"]),
            ("sitemap", &["xml"]),
        ])
    });

fn match_metadata_file<'a>(
    basename: &'a str,
    page_extensions: &[String],
) -> Option<(&'a str, i32, bool)> {
    let (stem, ext) = basename.split_once('.')?;
    static REGEX: Lazy<Regex> = Lazy::new(|| Regex::new("^(.*?)(\\d*)$").unwrap());
    let captures = REGEX.captures(stem).expect("the regex will always match");
    let stem = captures.get(1).unwrap().as_str();
    let num: i32 = captures.get(2).unwrap().as_str().parse().unwrap_or(-1);
    if page_extensions.iter().any(|e| e == ext) {
        return Some((stem, num, true));
    }
    let exts = STATIC_LOCAL_METADATA.get(stem)?;
    exts.contains(&ext).then_some((stem, num, false))
}

#[turbo_tasks::function]
async fn get_directory_tree(
    dir: Vc<FileSystemPath>,
    page_extensions: Vc<Vec<String>>,
) -> Result<Vc<DirectoryTree>> {
    let DirectoryContent::Entries(entries) = &*dir.read_dir().await? else {
        bail!("{} must be a directory", dir.to_string().await?);
    };
    let page_extensions_value = page_extensions.await?;

    let mut subdirectories = BTreeMap::new();
    let mut components = Components::default();

    let mut metadata_icon = Vec::new();
    let mut metadata_apple = Vec::new();
    let mut metadata_open_graph = Vec::new();
    let mut metadata_twitter = Vec::new();
    let mut metadata_favicon = Vec::new();

    for (basename, entry) in entries {
        match *entry {
            DirectoryEntry::File(file) => {
                if let Some((stem, ext)) = basename.split_once('.') {
                    if page_extensions_value.iter().any(|e| e == ext) {
                        match stem {
                            "page" => components.page = Some(file),
                            "layout" => components.layout = Some(file),
                            "error" => components.error = Some(file),
                            "loading" => components.loading = Some(file),
                            "template" => components.template = Some(file),
                            "not-found" => components.not_found = Some(file),
                            "default" => components.default = Some(file),
                            "route" => components.route = Some(file),
                            "manifest" => {
                                components.metadata.manifest =
                                    Some(MetadataItem::Dynamic { path: file });
                                continue;
                            }
                            _ => {}
                        }
                    }
                }

                if let Some((metadata_type, num, dynamic)) =
                    match_metadata_file(basename.as_str(), &page_extensions_value)
                {
                    if metadata_type == "manifest" {
                        if num == -1 {
                            components.metadata.manifest =
                                Some(MetadataItem::Static { path: file });
                        }
                        continue;
                    }

                    let entry = match metadata_type {
                        "icon" => Some(&mut metadata_icon),
                        "apple-icon" => Some(&mut metadata_apple),
                        "twitter-image" => Some(&mut metadata_twitter),
                        "opengraph-image" => Some(&mut metadata_open_graph),
                        "favicon" => Some(&mut metadata_favicon),
                        _ => None,
                    };

                    if let Some(entry) = entry {
                        if dynamic {
                            entry.push((num, MetadataWithAltItem::Dynamic { path: file }));
                        } else {
                            let file_value = file.await?;
                            let file_name = file_value.file_name();
                            let basename = file_name
                                .rsplit_once('.')
                                .map_or(file_name, |(basename, _)| basename);
                            let alt_path = file.parent().join(format!("{}.alt.txt", basename));
                            let alt_path =
                                matches!(&*alt_path.get_type().await?, FileSystemEntryType::File)
                                    .then_some(alt_path);
                            entry.push((
                                num,
                                MetadataWithAltItem::Static {
                                    path: file,
                                    alt_path,
                                },
                            ));
                        }
                    }
                }
            }
            DirectoryEntry::Directory(dir) => {
                // appDir ignores paths starting with an underscore
                if !basename.starts_with('_') {
                    let result = get_directory_tree(dir, page_extensions);
                    subdirectories.insert(get_underscore_normalized_path(basename), result);
                }
            }
            // TODO(WEB-952) handle symlinks in app dir
            _ => {}
        }
    }

    fn sort<T>(mut list: Vec<(i32, T)>) -> Vec<T> {
        list.sort_by_key(|(num, _)| *num);
        list.into_iter().map(|(_, item)| item).collect()
    }

    components.metadata.icon = sort(metadata_icon);
    components.metadata.apple = sort(metadata_apple);
    components.metadata.twitter = sort(metadata_twitter);
    components.metadata.open_graph = sort(metadata_open_graph);
    components.metadata.favicon = sort(metadata_favicon);

    Ok(DirectoryTree {
        subdirectories,
        components: components.cell(),
    }
    .cell())
}

#[turbo_tasks::value]
#[derive(Debug, Clone)]
pub struct LoaderTree {
    pub segment: String,
    pub parallel_routes: IndexMap<String, Vc<LoaderTree>>,
    pub components: Vc<Components>,
}

#[turbo_tasks::function]
async fn merge_loader_trees(
    app_dir: Vc<FileSystemPath>,
    tree1: Vc<LoaderTree>,
    tree2: Vc<LoaderTree>,
) -> Result<Vc<LoaderTree>> {
    let tree1 = tree1.await?;
    let tree2 = tree2.await?;

    let segment = if !tree1.segment.is_empty() {
        tree1.segment.to_string()
    } else {
        tree2.segment.to_string()
    };

    let mut parallel_routes = tree1.parallel_routes.clone();
    for (key, &tree2_route) in tree2.parallel_routes.iter() {
        add_parallel_route(app_dir, &mut parallel_routes, key.clone(), tree2_route).await?
    }

    let components = Components::merge(&*tree1.components.await?, &*tree2.components.await?).cell();

    Ok(LoaderTree {
        segment,
        parallel_routes,
        components,
    }
    .cell())
}

#[derive(
    Clone, PartialEq, Eq, Serialize, Deserialize, TraceRawVcs, ValueDebugFormat, Debug, TaskInput,
)]
pub enum Entrypoint {
    AppPage {
        original_name: String,
        loader_tree: Vc<LoaderTree>,
    },
    AppRoute {
        original_name: String,
        path: Vc<FileSystemPath>,
    },
}

#[turbo_tasks::value(transparent)]
pub struct Entrypoints(IndexMap<String, Entrypoint>);

fn is_parallel_route(name: &str) -> bool {
    name.starts_with('@')
}

fn match_parallel_route(name: &str) -> Option<&str> {
    name.strip_prefix('@')
}

async fn add_parallel_route(
    app_dir: Vc<FileSystemPath>,
    result: &mut IndexMap<String, Vc<LoaderTree>>,
    key: String,
    loader_tree: Vc<LoaderTree>,
) -> Result<()> {
    match result.entry(key) {
        Entry::Occupied(mut e) => {
            let value = e.get_mut();
            *value = merge_loader_trees(app_dir, *value, loader_tree)
                .resolve()
                .await?;
        }
        Entry::Vacant(e) => {
            e.insert(loader_tree);
        }
    }
    Ok(())
}

async fn add_app_page(
    app_dir: Vc<FileSystemPath>,
    result: &mut IndexMap<String, Entrypoint>,
    key: String,
    original_name: String,
    loader_tree: Vc<LoaderTree>,
) -> Result<()> {
    match result.entry(key) {
        Entry::Occupied(mut e) => {
            let value = e.get();
            match value {
                Entrypoint::AppPage {
                    original_name: existing_original_name,
                    ..
                } => {
                    if *existing_original_name != original_name {
                        DirectoryTreeIssue {
                            app_dir,
                            message: Vc::cell(format!(
                                "Conflicting pages at {}: {existing_original_name} and \
                                 {original_name}",
                                e.key()
                            )),
                            severity: IssueSeverity::Error.cell(),
                        }
                        .cell()
                        .emit();
                        return Ok(());
                    }
                    if let Entrypoint::AppPage {
                        loader_tree: value, ..
                    } = e.get_mut()
                    {
                        *value = merge_loader_trees(app_dir, *value, loader_tree)
                            .resolve()
                            .await?;
                    }
                }
                Entrypoint::AppRoute {
                    original_name: existing_original_name,
                    ..
                } => {
                    DirectoryTreeIssue {
                        app_dir,
                        message: Vc::cell(format!(
                            "Conflicting page and route at {}: route at {existing_original_name} \
                             and page at {original_name}",
                            e.key()
                        )),
                        severity: IssueSeverity::Error.cell(),
                    }
                    .cell()
                    .emit();
                    return Ok(());
                }
            }
        }
        Entry::Vacant(e) => {
            e.insert(Entrypoint::AppPage {
                original_name,
                loader_tree,
            });
        }
    }
    Ok(())
}

async fn add_app_route(
    app_dir: Vc<FileSystemPath>,
    result: &mut IndexMap<String, Entrypoint>,
    key: String,
    original_name: String,
    path: Vc<FileSystemPath>,
) -> Result<()> {
    match result.entry(key) {
        Entry::Occupied(mut e) => {
            let value = e.get();
            match value {
                Entrypoint::AppPage {
                    original_name: existing_original_name,
                    ..
                } => {
                    DirectoryTreeIssue {
                        app_dir,
                        message: Vc::cell(format!(
                            "Conflicting route and page at {}: route at {original_name} and page \
                             at {existing_original_name}",
                            e.key()
                        )),
                        severity: IssueSeverity::Error.cell(),
                    }
                    .cell()
                    .emit();
                }
                Entrypoint::AppRoute {
                    original_name: existing_original_name,
                    ..
                } => {
                    DirectoryTreeIssue {
                        app_dir,
                        message: Vc::cell(format!(
                            "Conflicting routes at {}: {existing_original_name} and \
                             {original_name}",
                            e.key()
                        )),
                        severity: IssueSeverity::Error.cell(),
                    }
                    .cell()
                    .emit();
                    return Ok(());
                }
            }
            *e.get_mut() = Entrypoint::AppRoute {
                original_name,
                path,
            };
        }
        Entry::Vacant(e) => {
            e.insert(Entrypoint::AppRoute {
                original_name,
                path,
            });
        }
    }
    Ok(())
}

#[turbo_tasks::function]
pub fn get_entrypoints(
    app_dir: Vc<FileSystemPath>,
    page_extensions: Vc<Vec<String>>,
) -> Vc<Entrypoints> {
    directory_tree_to_entrypoints(app_dir, get_directory_tree(app_dir, page_extensions))
}

#[turbo_tasks::function]
fn directory_tree_to_entrypoints(
    app_dir: Vc<FileSystemPath>,
    directory_tree: Vc<DirectoryTree>,
) -> Vc<Entrypoints> {
    directory_tree_to_entrypoints_internal(
        app_dir,
        "".to_string(),
        directory_tree,
        "/".to_string(),
        "/".to_string(),
    )
}

#[turbo_tasks::function]
async fn directory_tree_to_entrypoints_internal(
    app_dir: Vc<FileSystemPath>,
    directory_name: String,
    directory_tree: Vc<DirectoryTree>,
    path_prefix: String,
    original_name_prefix: String,
) -> Result<Vc<Entrypoints>> {
    let mut result = IndexMap::new();

    let directory_tree = &*directory_tree.await?;

    let subdirectories = &directory_tree.subdirectories;
    let components = directory_tree.components.await?;

    let current_level_is_parallel_route = is_parallel_route(&directory_name);

    if let Some(page) = components.page {
        add_app_page(
            app_dir,
            &mut result,
            path_prefix.to_string(),
            original_name_prefix.to_string(),
            if current_level_is_parallel_route {
                LoaderTree {
                    segment: "__PAGE__".to_string(),
                    parallel_routes: IndexMap::new(),
                    components: Components {
                        page: Some(page),
                        ..Default::default()
                    }
                    .cell(),
                }
                .cell()
            } else {
                LoaderTree {
                    segment: directory_name.to_string(),
                    parallel_routes: indexmap! {
                        "children".to_string() => LoaderTree {
                            segment: "__PAGE__".to_string(),
                            parallel_routes: IndexMap::new(),
                            components: Components {
                                page: Some(page),
                                ..Default::default()
                            }
                            .cell(),
                        }
                        .cell(),
                    },
                    components: components.without_leafs().cell(),
                }
                .cell()
            },
        )
        .await?;
    }

    if let Some(default) = components.default {
        add_app_page(
            app_dir,
            &mut result,
            path_prefix.to_string(),
            original_name_prefix.to_string(),
            if current_level_is_parallel_route {
                LoaderTree {
                    segment: "__DEFAULT__".to_string(),
                    parallel_routes: IndexMap::new(),
                    components: Components {
                        default: Some(default),
                        ..Default::default()
                    }
                    .cell(),
                }
                .cell()
            } else {
                LoaderTree {
                    segment: directory_name.to_string(),
                    parallel_routes: indexmap! {
                        "children".to_string() => LoaderTree {
                            segment: "__DEFAULT__".to_string(),
                            parallel_routes: IndexMap::new(),
                            components: Components {
                                default: Some(default),
                                ..Default::default()
                            }
                            .cell(),
                        }
                        .cell(),
                    },
                    components: components.without_leafs().cell(),
                }
                .cell()
            },
        )
        .await?;
    }

    if let Some(route) = components.route {
        add_app_route(
            app_dir,
            &mut result,
            path_prefix.to_string(),
            original_name_prefix.to_string(),
            route,
        )
        .await?;
    }

    if path_prefix == "/" {
        // Next.js has this logic in "collect-app-paths", where the root not-found page
        // is considered as its own entry point.
        if let Some(_not_found) = components.not_found {
            let tree = LoaderTree {
                segment: directory_name.to_string(),
                parallel_routes: indexmap! {
                    "children".to_string() => LoaderTree {
                        segment: "__DEFAULT__".to_string(),
                        parallel_routes: IndexMap::new(),
                        components: Components {
                            default: Some(get_next_package(app_dir).join("dist/client/components/parallel-route-default.js".to_string())),
                            ..Default::default()
                        }
                        .cell(),
                    }
                    .cell(),
                },
                components: components.without_leafs().cell(),
            }
            .cell();
            add_app_page(
                app_dir,
                &mut result,
                "/not-found".to_string(),
                "/not-found".to_string(),
                tree,
            )
            .await?;
            add_app_page(
                app_dir,
                &mut result,
                "/_not-found".to_string(),
                "/_not-found".to_string(),
                tree,
            )
            .await?;
        }
    }

    for (subdir_name, &subdirectory) in subdirectories.iter() {
        let is_route_group = subdir_name.starts_with('(') && subdir_name.ends_with(')');
        let parallel_route_key = match_parallel_route(subdir_name);
        let map = directory_tree_to_entrypoints_internal(
            app_dir,
            subdir_name.to_string(),
            subdirectory,
            if is_route_group || parallel_route_key.is_some() {
                path_prefix.clone()
            } else if path_prefix == "/" {
                format!("/{subdir_name}")
            } else {
                format!("{path_prefix}/{subdir_name}")
            },
            if is_route_group || parallel_route_key.is_some() {
                path_prefix.clone()
            } else if path_prefix == "/" {
                format!("/{subdir_name}")
            } else {
                format!("{path_prefix}/{subdir_name}")
            },
        )
        .await?;
        for (full_path, entrypoint) in map.iter() {
            match *entrypoint {
                Entrypoint::AppPage {
                    ref original_name,
                    loader_tree,
                } => {
                    if current_level_is_parallel_route {
                        add_app_page(
                            app_dir,
                            &mut result,
                            full_path.clone(),
                            original_name.clone(),
                            loader_tree,
                        )
                        .await?;
                    } else {
                        let key = parallel_route_key.unwrap_or("children").to_string();
                        let child_loader_tree = LoaderTree {
                            segment: directory_name.to_string(),
                            parallel_routes: indexmap! {
                                key => loader_tree,
                            },
                            components: components.without_leafs().cell(),
                        }
                        .cell();
                        add_app_page(
                            app_dir,
                            &mut result,
                            full_path.clone(),
                            original_name.clone(),
                            child_loader_tree,
                        )
                        .await?;
                    }
                }
                Entrypoint::AppRoute {
                    ref original_name,
                    path,
                } => {
                    add_app_route(
                        app_dir,
                        &mut result,
                        full_path.clone(),
                        original_name.clone(),
                        path,
                    )
                    .await?;
                }
            }
        }
    }
    Ok(Vc::cell(result))
}

/// ref: https://github.com/vercel/next.js/blob/c390c1662bc79e12cf7c037dcb382ef5ead6e492/packages/next/src/build/entries.ts#L119
/// if path contains %5F, replace it with _.
fn get_underscore_normalized_path(path: &str) -> String {
    path.replace("%5F", "_")
}

/// Returns the global metadata for an app directory.
#[turbo_tasks::function]
pub async fn get_global_metadata(
    app_dir: Vc<FileSystemPath>,
    page_extensions: Vc<Vec<String>>,
) -> Result<Vc<GlobalMetadata>> {
    let DirectoryContent::Entries(entries) = &*app_dir.read_dir().await? else {
        bail!("app_dir must be a directory")
    };
    let mut metadata = GlobalMetadata::default();

    for (basename, entry) in entries {
        if let DirectoryEntry::File(file) = *entry {
            if let Some((stem, ext)) = basename.split_once('.') {
                let list = match stem {
                    "favicon" => Some(&mut metadata.favicon),
                    "sitemap" => Some(&mut metadata.sitemap),
                    "robots" => Some(&mut metadata.robots),
                    _ => None,
                };
                if let Some(list) = list {
                    if page_extensions.await?.iter().any(|e| e == ext) {
                        *list = Some(MetadataItem::Dynamic { path: file });
                    }
                    if STATIC_GLOBAL_METADATA.get(stem).unwrap().contains(&ext) {
                        *list = Some(MetadataItem::Static { path: file });
                    }
                }
            }
        }
        // TODO(WEB-952) handle symlinks in app dir
    }

    Ok(metadata.cell())
}

#[turbo_tasks::value(shared)]
struct DirectoryTreeIssue {
    pub severity: Vc<IssueSeverity>,
    pub app_dir: Vc<FileSystemPath>,
    pub message: Vc<String>,
}

#[turbo_tasks::value_impl]
impl Issue for DirectoryTreeIssue {
    #[turbo_tasks::function]
    fn severity(&self) -> Vc<IssueSeverity> {
        self.severity
    }

    #[turbo_tasks::function]
    async fn title(&self) -> Result<Vc<String>> {
        Ok(Vc::cell(
            "An issue occurred while preparing your Next.js app".to_string(),
        ))
    }

    #[turbo_tasks::function]
    fn category(&self) -> Vc<String> {
        Vc::cell("next app".to_string())
    }

    #[turbo_tasks::function]
    fn file_path(&self) -> Vc<FileSystemPath> {
        self.app_dir
    }

    #[turbo_tasks::function]
    fn description(&self) -> Vc<String> {
        self.message
    }
}
