#![allow(dead_code)]

use std::{
    borrow::Cow,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use actix_files::{Files, NamedFile};
use actix_service::fn_service;
use actix_web::dev::{HttpServiceFactory, ResourceDef, ServiceRequest, ServiceResponse};
use glob::glob;
use path_tree::PathTree;
use regex::{Captures, Regex};
use tracing::{trace, warn};

/// Single Page App (SPA) service builder.
///
/// # Examples
/// ```
/// # use actix_web::App;
/// # use actix_web_nextjs_spa::spa;
/// let app = App::new()
///     // ...api routes...
///     .service(
///         spa()
///             .index_file("./examples/assets/spa.html")
///             .static_resources_mount("/static")
///             .static_resources_location("./examples/assets")
///             .finish()
///     );
/// ```
#[cfg_attr(docsrs, doc(cfg(feature = "spa")))]
#[derive(Debug, Clone)]
pub struct Spa {
    index_file: Cow<'static, str>,
    static_resources_mount: Cow<'static, str>,
    static_resources_location: Cow<'static, str>,
}

impl Spa {
    /// Location of the SPA index file.
    ///
    /// This file will be served if:
    /// - the Actix Web router has reached this service, indicating that none of the API routes
    ///   matched the URL path;
    /// - and none of the static resources handled matched.
    ///
    /// The default is "./index.html". I.e., the `index.html` file located in the directory that
    /// the server is running from.
    pub fn index_file(mut self, index_file: impl Into<Cow<'static, str>>) -> Self {
        self.index_file = index_file.into();
        self
    }

    /// The URL path prefix that static files should be served from.
    ///
    /// The default is "/". I.e., static files are served from the root URL path.
    pub fn static_resources_mount(
        mut self,
        static_resources_mount: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.static_resources_mount = static_resources_mount.into();
        self
    }

    /// The location in the filesystem to serve static resources from.
    ///
    /// The default is "./". I.e., static files are located in the directory the server is
    /// running from.
    pub fn static_resources_location(
        mut self,
        static_resources_location: impl Into<Cow<'static, str>>,
    ) -> Self {
        self.static_resources_location = static_resources_location.into();
        self
    }

    /// Constructs the service for use in a `.service()` call.
    pub fn finish(self) -> impl HttpServiceFactory {
        let index_file = self.index_file.into_owned();
        let static_resources_location = self.static_resources_location.into_owned();
        let static_resources_location_clone = static_resources_location.clone();
        let static_resources_mount = self.static_resources_mount.into_owned();

        let files = {
            let index_file = index_file.clone();

            let path_tree = Arc::new(
                find_build_manifest(static_resources_location.clone())
                    .and_then(|build_manifest_path| fs::read_to_string(build_manifest_path).ok())
                    .and_then(|build_manifest_content| {
                        Some(parse_build_manifest(
                            build_manifest_content,
                            &static_resources_location,
                        ))
                    })
                    .unwrap_or(PathTree::default()),
            );

            Files::new(&static_resources_mount, static_resources_location)
                // HACK: FilesService will try to read a directory listing unless index_file is provided
                // FilesService will fail to load the index_file and will then call our default_handler
                .index_file("extremely-unlikely-to-exist-!@$%^&*.txt")
                .default_handler(move |req| serve_index(req, index_file.clone(), path_tree.clone()))
        };

        SpaService {
            index_file,
            static_resources_location: static_resources_location_clone.clone(),
            files,
        }
    }
}

#[derive(Debug)]
struct SpaService {
    index_file: String,
    static_resources_location: String,
    files: Files,
}

impl HttpServiceFactory for SpaService {
    fn register(self, config: &mut actix_web::dev::AppService) {
        // let Files register its mount path as-is
        self.files.register(config);

        let path_tree = Arc::new(
            find_build_manifest(self.static_resources_location.clone())
                .and_then(|build_manifest_path| fs::read_to_string(build_manifest_path).ok())
                .and_then(|build_manifest_content| {
                    Some(parse_build_manifest(
                        build_manifest_content,
                        &self.static_resources_location,
                    ))
                })
                .unwrap_or(PathTree::default()),
        );

        // also define a root prefix handler directed towards our SPA index
        let rdef = ResourceDef::root_prefix("");
        config.register_service(
            rdef,
            None,
            fn_service(move |req| {
                trace!("building tree path");

                serve_index(req, self.index_file.clone(), path_tree.clone())
            }),
            None,
        );
    }
}

async fn serve_index(
    req: ServiceRequest,
    index_file: String,
    path_tree: Arc<PathTree<String>>,
) -> Result<ServiceResponse, actix_web::Error> {
    trace!("serving default SPA page");
    let (req, _) = req.into_parts();

    let file = match path_tree.find(req.path()) {
        Some((h, _)) => match NamedFile::open_async(h).await {
            Ok(f) => Ok(f),
            Err(e) => match e.kind() {
                ErrorKind::NotFound => NamedFile::open_async(&index_file).await,
                _ => Err(e),
            },
        },
        None => NamedFile::open_async(&index_file).await,
    }?;

    let res = file.into_response(&req);
    Ok(ServiceResponse::new(req, res))
}

fn find_build_manifest(static_resources_location: String) -> Option<PathBuf> {
    let pattern = format!("{}/_next/**/_buildManifest.js", static_resources_location);
    let entries = glob(&pattern);

    match entries {
        Ok(paths) => {
            for path in paths {
                match path {
                    Ok(p) => {
                        return Some(p);
                    }
                    Err(err) => {
                        warn!("{}", err);
                        return None;
                    }
                }
            }

            warn!("_buildManifest.js not found");
            return None;
        }
        Err(err) => {
            warn!("{}", err);
            return None;
        }
    }
}

fn parse_build_manifest(
    build_manifest: String,
    static_resources_location: &str,
) -> PathTree<String> {
    let re = Regex::new(r#""([^,]+)":\s*\["[^,]+"\]"#).unwrap();
    let mut tree = PathTree::new();

    let resources_path = Path::new(static_resources_location);

    for (_, [path]) in re.captures_iter(&build_manifest).map(|c| c.extract()) {
        let value = resources_path
            .join(format!(
                "{}.html",
                if path == "/" {
                    "index"
                } else {
                    path.strip_prefix("/").unwrap()
                }
            ))
            .to_str()
            .unwrap()
            .to_string();
        let path = convert_dynamic_path(path).replace(".html", "");

        let _ = tree.insert(&path, value);
    }

    tree
}

fn convert_dynamic_path(path: &str) -> String {
    let re = Regex::new(r#"(?<param>\[[^\]]+\])"#).unwrap();
    return re
        .replace_all(path, |caps: &Captures| {
            format!(":{}", &caps[1].replace("[", "").replace("]", ""))
        })
        .to_string();
}

impl Default for Spa {
    fn default() -> Self {
        Self {
            index_file: Cow::Borrowed("./index.html"),
            static_resources_mount: Cow::Borrowed("/"),
            static_resources_location: Cow::Borrowed("./"),
        }
    }
}

pub fn spa() -> Spa {
    Spa::default()
}

#[cfg(test)]
mod tests {
    use std::str::from_utf8;

    use actix_web::{body::MessageBody, dev::ServiceFactory, http::StatusCode, test, App, Error};

    use super::*;

    fn test_app() -> App<
        impl ServiceFactory<
            ServiceRequest,
            Response = ServiceResponse<impl MessageBody>,
            Config = (),
            InitError = (),
            Error = Error,
        >,
    > {
        App::new().service(
            Spa::default()
                .index_file("./fixtures/001/index.html")
                .static_resources_location("./fixtures/001")
                .finish(),
        )
    }

    #[actix_web::test]
    async fn returns_index() {
        let app = test::init_service(test_app()).await;

        let req = test::TestRequest::default().to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::OK);

        let body = test::read_body(res).await;
        let html = from_utf8(&body).unwrap();
        assert!(html.contains("Home page"));
    }

    #[actix_web::test]
    async fn returns_page() {
        let app = test::init_service(test_app()).await;

        let req = test::TestRequest::default().uri("/page").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::OK);

        let body = test::read_body(res).await;
        let html = from_utf8(&body).unwrap();
        assert!(html.contains("Sample Page"));
    }

    #[actix_web::test]
    async fn returns_item_page() {
        let app = test::init_service(test_app()).await;

        let req = test::TestRequest::default()
            .uri("/dog/items/cat")
            .to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::OK);

        let body = test::read_body(res).await;
        let html = from_utf8(&body).unwrap();
        assert!(html.contains("Item Page"));
    }

    #[actix_web::test]
    async fn unknown_page_returns_index() {
        let app = test::init_service(test_app()).await;

        let req = test::TestRequest::default().uri("/whatisthis").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::OK);

        let body = test::read_body(res).await;
        let html = from_utf8(&body).unwrap();
        assert!(html.contains("Home page"));
    }

    #[actix_web::test]
    async fn returns_assets() {
        let app = test::init_service(test_app()).await;

        let req = test::TestRequest::default().uri("/next.svg").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::OK);

        let body = test::read_body(res).await;
        let svg = from_utf8(&body).unwrap();
        assert!(svg.contains(r#"<svg xmlns="http://www.w3.org/2000/svg" fill="none""#));
    }
}
