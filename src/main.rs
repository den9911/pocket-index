mod indexer;
mod query;
mod startup;
use std::{net::{IpAddr,Ipv4Addr,SocketAddr},path::{Path,PathBuf},sync::{Arc,Mutex}};
use anyhow::Result;
use axum::{extract::{Query,State},http::{header,HeaderMap,StatusCode},response::{Html,IntoResponse},routing::{get,post},Json,Router};
use rusqlite::Connection;
use serde::{Deserialize,Serialize};
use indexer::{active_index, database as db, run_scan, Progress};
#[cfg(test)]
use indexer::scan;

#[derive(Clone)]
struct App {db:PathBuf,progress:Arc<Mutex<Progress>>}
#[derive(Deserialize)]
struct IndexRequest {path:String}
#[derive(Deserialize)]
struct SearchRequest {q:Option<String>,#[serde(default)] mode:query::Mode}
#[derive(Serialize)]
struct Hit {title:String,path:String,excerpt:String}
#[cfg(test)]
fn search(path: &Path, q: &str) -> Result<Vec<Hit>> {
    search_mode(path, q, query::Mode::All)
}
#[cfg(test)]
fn search_mode(path: &Path, q: &str, mode: query::Mode) -> Result<Vec<Hit>> {
    search_snapshot(path, q, mode).map(|(hits, _)| hits)
}
fn search_snapshot(path: &Path, q: &str, mode: query::Mode) -> Result<(Vec<Hit>, u64)> {
    let expression = query::expression(q, mode)?;
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    let generation = transaction.query_row("SELECT generation FROM pocket_index_state WHERE singleton=1", [], |r| r.get(0))?;
    let hits = if expression.is_empty() {
        Vec::new()
    } else {
        let mut statement = transaction.prepare("SELECT title,path,snippet(docs,2,'','', ' … ',24) FROM docs WHERE docs MATCH ?1 ORDER BY bm25(docs,0.0,3.0,1.0) LIMIT 50")?;
        let rows = statement.query_map([expression], |r| Ok(Hit { title:r.get(0)?, path:r.get(1)?, excerpt:r.get(2)? }))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok((hits, generation))
}
async fn status(State(app):State<App>)->Result<Json<Progress>,StatusCode>{app.progress.lock().map(|p|Json(p.clone())).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)}
async fn query(State(app): State<App>, Query(req): Query<SearchRequest>) -> Result<impl IntoResponse, StatusCode> {
    let q = req.q.unwrap_or_default();
    if query::expression(&q, req.mode).is_err() { return Err(StatusCode::BAD_REQUEST); }
    let mode = req.mode;
    let path = app.db.clone();
    let (hits, generation) = tokio::task::spawn_blocking(move || search_snapshot(&path, &q, mode))
        .await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(([("x-pocket-index-generation", generation.to_string())], Json(hits)))
}
async fn index(State(app):State<App>,headers:HeaderMap,Json(req):Json<IndexRequest>)->Result<StatusCode,StatusCode>{
 if headers.get("x-pocket-request").and_then(|v|v.to_str().ok())!=Some("1"){return Err(StatusCode::FORBIDDEN)}
 if req.path.trim().is_empty()||req.path.len()>4096{return Err(StatusCode::BAD_REQUEST)}
 {let mut p=app.progress.lock().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;if p.phase=="running"{return Err(StatusCode::CONFLICT)}let active=p.active.clone();*p=Progress{phase:"running".into(),active,..Progress::default()};}
 let folder=PathBuf::from(req.path);let db_path=app.db.clone();let progress=app.progress.clone();
 tokio::task::spawn_blocking(move||run_scan(&folder,&db_path,&progress));
 Ok(StatusCode::ACCEPTED)
}
async fn index_page()->Html<&'static str>{Html(include_str!("../web/index.html"))}
async fn style()->impl IntoResponse{([(header::CONTENT_TYPE,"text/css; charset=utf-8")],include_str!("../web/style.css"))}
async fn script()->impl IntoResponse{([(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../web/app.js"))}
#[tokio::main]
async fn main()->Result<()>{let path=std::env::var_os("POCKET_INDEX_DB").map(PathBuf::from).unwrap_or_else(||PathBuf::from("pocket-index.sqlite"));if std::env::args().any(|a|a=="--check"){return startup::check(Some(&path),Some(3002));}let active=active_index(&db(&path)?)?;
 let app=App{db:path,progress:Arc::new(Mutex::new(Progress{phase:"idle".into(),active,..Progress::default()}))};
 let router=Router::new().route("/",get(index_page)).route("/style.css",get(style)).route("/app.js",get(script)).route("/api/status",get(status)).route("/api/search",get(query)).route("/api/index",post(index)).with_state(app);
 let addr=SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST),3002);let listener=tokio::net::TcpListener::bind(addr).await.map_err(|_|anyhow::anyhow!("Cannot bind local port; stop the other instance / Локальный порт занят или недоступен"))?;println!("Pocket Index: http://{addr}");axum::serve(listener,router).with_graceful_shutdown(async{let _=tokio::signal::ctrl_c().await;}).await?;Ok(())}
#[cfg(test)]
mod tests {use super::*;
 #[test]fn indexes_nested_and_persists()->Result<()>{let tmp=tempfile::tempdir()?;let folder=tmp.path().join("notes");std::fs::create_dir_all(folder.join("nested"))?;std::fs::write(folder.join("nested/a.md"),"Rust systems and привет мир")?;std::fs::write(folder.join("ignore.jpg"),"no")?;let path=tmp.path().join("index.db");let p=Arc::new(Mutex::new(Progress::default()));scan(&folder,&path,&p)?;assert_eq!(search(&path,"привет")?.len(),1);assert_eq!(search(&path,"Rust")?.len(),1);assert!(search(&path,"missing")?.is_empty());std::fs::write(folder.join("nested/a.md"),"changed note")?;scan(&folder,&path,&p)?;assert!(search(&path,"привет")?.is_empty());assert_eq!(search(&path,"changed")?.len(),1);Ok(())}
}

#[cfg(test)]mod mode_tests{use super::*;
 #[test]fn phrase_and_prefix_match_real_fts()->Result<()>{let tmp=tempfile::tempdir()?;let root=tmp.path().join("notes");std::fs::create_dir(&root)?;std::fs::write(root.join("one.md"),"rust systems привет мир")?;std::fs::write(root.join("two.md"),"systems with rust приветствие")?;let path=tmp.path().join("test.db");scan(&root,&path,&Arc::new(Mutex::new(Progress::default())))?;assert_eq!(search_mode(&path,"rust systems",query::Mode::Phrase)?.len(),1);assert_eq!(search_mode(&path,"rust systems",query::Mode::All)?.len(),2);assert_eq!(search_mode(&path,"прив",query::Mode::Prefix)?.len(),2);Ok(())}
}
