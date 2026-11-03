mod query;
mod startup;
use std::{net::{IpAddr,Ipv4Addr,SocketAddr},path::{Path,PathBuf},sync::{Arc,Mutex}};
use anyhow::{Context,Result};
use axum::{extract::{Query,State},http::{header,HeaderMap,StatusCode},response::{Html,IntoResponse},routing::{get,post},Json,Router};
use rusqlite::{params,Connection};
use serde::{Deserialize,Serialize};
use walkdir::WalkDir;

const MAX_BYTES:u64=1_048_576;
#[derive(Clone,Default,Serialize)]
struct Progress {phase:String,found:usize,processed:usize,skipped:usize,errors:usize,message:String}
#[derive(Clone)]
struct App {db:PathBuf,progress:Arc<Mutex<Progress>>}
#[derive(Deserialize)]
struct IndexRequest {path:String}
#[derive(Deserialize)]
struct SearchRequest {q:Option<String>,#[serde(default)] mode:query::Mode}
#[derive(Serialize)]
struct Hit {title:String,path:String,excerpt:String}
fn db(path:&Path)->Result<Connection>{let c=Connection::open(path)?;c.execute_batch("PRAGMA journal_mode=WAL; CREATE VIRTUAL TABLE IF NOT EXISTS docs USING fts5(path UNINDEXED,title,body,tokenize='unicode61');")?;Ok(c)}
fn set_progress(state:&Arc<Mutex<Progress>>,f:impl FnOnce(&mut Progress)){if let Ok(mut p)=state.lock(){f(&mut p)}}
fn scan(root:&Path,db_path:&Path,progress:&Arc<Mutex<Progress>>)->Result<()> {
 let root=root.canonicalize().context("Cannot open folder")?; anyhow::ensure!(root.is_dir(),"Path must be a folder");
 let mut conn=db(db_path)?; let tx=conn.transaction()?;tx.execute("DELETE FROM docs",[])?;
 for entry in WalkDir::new(&root).follow_links(false).into_iter(){
  let e=match entry{Ok(e)=>e,Err(_)=>{set_progress(progress,|p|p.errors+=1);continue}};
  if !e.file_type().is_file(){continue}
  let extension=e.path().extension().and_then(|x|x.to_str()).unwrap_or("").to_ascii_lowercase();
  if extension!="md" && extension!="txt" {set_progress(progress,|p|p.skipped+=1);continue}
  set_progress(progress,|p|p.found+=1);
  let meta=match e.metadata(){Ok(x)=>x,Err(_)=>{set_progress(progress,|p|p.errors+=1);continue}};
  if meta.len()>MAX_BYTES{set_progress(progress,|p|p.skipped+=1);continue}
  let text=match std::fs::read_to_string(e.path()){Ok(s)=>s,Err(_)=>{set_progress(progress,|p|p.errors+=1);continue}};
  let path=e.path().strip_prefix(&root)?.to_string_lossy().to_string();let title=e.file_name().to_string_lossy().to_string();
  tx.execute("INSERT INTO docs(path,title,body) VALUES(?1,?2,?3)",params![path,title,text])?;
  set_progress(progress,|p|p.processed+=1);
 }
 tx.commit()?; Ok(())
}
#[cfg(test)]
fn search(path:&Path,q:&str)->Result<Vec<Hit>>{search_mode(path,q,query::Mode::All)}
fn search_mode(path:&Path,q:&str,mode:query::Mode)->Result<Vec<Hit>>{
 let query=query::expression(q,mode)?;
 if query.is_empty(){return Ok(vec![])}
 let c=db(path)?;let mut stmt=c.prepare("SELECT title,path,snippet(docs,2,'','', ' … ',24) FROM docs WHERE docs MATCH ?1 ORDER BY bm25(docs,0.0,3.0,1.0) LIMIT 50")?;
 let rows=stmt.query_map([query],|r|Ok(Hit{title:r.get(0)?,path:r.get(1)?,excerpt:r.get(2)?}))?;Ok(rows.collect::<rusqlite::Result<_>>()?)
}
async fn status(State(app):State<App>)->Json<Progress>{Json(app.progress.lock().map(|p|p.clone()).unwrap_or_default())}
async fn query(State(app):State<App>,Query(req):Query<SearchRequest>)->Result<Json<Vec<Hit>>,StatusCode>{let q=req.q.unwrap_or_default();if query::expression(&q,req.mode).is_err(){return Err(StatusCode::BAD_REQUEST)}let mode=req.mode;let path=app.db.clone();tokio::task::spawn_blocking(move||search_mode(&path,&q,mode)).await.map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?.map(Json).map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)}
async fn index(State(app):State<App>,headers:HeaderMap,Json(req):Json<IndexRequest>)->Result<StatusCode,StatusCode>{
 if headers.get("x-pocket-request").and_then(|v|v.to_str().ok())!=Some("1"){return Err(StatusCode::FORBIDDEN)}
 if req.path.trim().is_empty()||req.path.len()>4096{return Err(StatusCode::BAD_REQUEST)}
 {let mut p=app.progress.lock().map_err(|_|StatusCode::INTERNAL_SERVER_ERROR)?;if p.phase=="running"{return Err(StatusCode::CONFLICT)}*p=Progress{phase:"running".into(),..Progress::default()};}
 let folder=PathBuf::from(req.path);let db_path=app.db.clone();let progress=app.progress.clone();
 tokio::task::spawn_blocking(move||{let result=scan(&folder,&db_path,&progress);set_progress(&progress,|p|match result{Ok(())=>{p.phase=if p.errors>0{"partial"}else{"complete"}.into()},Err(e)=>{p.phase="error".into();p.message=e.to_string()}});});
 Ok(StatusCode::ACCEPTED)
}
async fn index_page()->Html<&'static str>{Html(include_str!("../web/index.html"))}
async fn style()->impl IntoResponse{([(header::CONTENT_TYPE,"text/css; charset=utf-8")],include_str!("../web/style.css"))}
async fn script()->impl IntoResponse{([(header::CONTENT_TYPE,"text/javascript; charset=utf-8")],include_str!("../web/app.js"))}
#[tokio::main]
async fn main()->Result<()>{let path=std::env::var_os("POCKET_INDEX_DB").map(PathBuf::from).unwrap_or_else(||PathBuf::from("pocket-index.sqlite"));if std::env::args().any(|a|a=="--check"){return startup::check(Some(&path),Some(3002));}db(&path)?;
 let app=App{db:path,progress:Arc::new(Mutex::new(Progress{phase:"idle".into(),..Progress::default()}))};
 let router=Router::new().route("/",get(index_page)).route("/style.css",get(style)).route("/app.js",get(script)).route("/api/status",get(status)).route("/api/search",get(query)).route("/api/index",post(index)).with_state(app);
 let addr=SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST),3002);let listener=tokio::net::TcpListener::bind(addr).await.map_err(|_|anyhow::anyhow!("Cannot bind local port; stop the other instance / Локальный порт занят или недоступен"))?;println!("Pocket Index: http://{addr}");axum::serve(listener,router).with_graceful_shutdown(async{let _=tokio::signal::ctrl_c().await;}).await?;Ok(())}
#[cfg(test)]
mod tests {use super::*;
 #[test]fn indexes_nested_and_persists()->Result<()>{let tmp=tempfile::tempdir()?;let folder=tmp.path().join("notes");std::fs::create_dir_all(folder.join("nested"))?;std::fs::write(folder.join("nested/a.md"),"Rust systems and привет мир")?;std::fs::write(folder.join("ignore.jpg"),"no")?;let path=tmp.path().join("index.db");let p=Arc::new(Mutex::new(Progress::default()));scan(&folder,&path,&p)?;assert_eq!(search(&path,"привет")?.len(),1);assert_eq!(search(&path,"Rust")?.len(),1);assert!(search(&path,"missing")?.is_empty());std::fs::write(folder.join("nested/a.md"),"changed note")?;scan(&folder,&path,&p)?;assert!(search(&path,"привет")?.is_empty());assert_eq!(search(&path,"changed")?.len(),1);Ok(())}
}

#[cfg(test)]mod mode_tests{use super::*;
 #[test]fn phrase_and_prefix_match_real_fts()->Result<()>{let tmp=tempfile::tempdir()?;let root=tmp.path().join("notes");std::fs::create_dir(&root)?;std::fs::write(root.join("one.md"),"rust systems привет мир")?;std::fs::write(root.join("two.md"),"systems with rust приветствие")?;let path=tmp.path().join("test.db");scan(&root,&path,&Arc::new(Mutex::new(Progress::default())))?;assert_eq!(search_mode(&path,"rust systems",query::Mode::Phrase)?.len(),1);assert_eq!(search_mode(&path,"rust systems",query::Mode::All)?.len(),2);assert_eq!(search_mode(&path,"прив",query::Mode::Prefix)?.len(),2);Ok(())}
}
