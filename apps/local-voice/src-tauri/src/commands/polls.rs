//! TF2: Nutzeraktionen; ausschliesslich poll_send verschickt Anfrage-Mails.
use std::sync::Arc;
use tauri::{AppHandle, State};
use besprechungsplanung::terminfindung::AntwortWert;
use crate::managers::integrations::m365::M365Service;
use crate::managers::integrations::m365::polls::{PersonAvailability, ScheduleWindow};
use crate::managers::meetings::{polls::{self, PollDefinition, PollRuntime, PollView}, store::MeetingStore};

type Meetings<'a> = State<'a,Arc<MeetingStore>>;
type Service<'a> = State<'a,Arc<M365Service>>;
type Runtime<'a> = State<'a,Arc<PollRuntime>>;

fn conn(store: &MeetingStore) -> Result<rusqlite::Connection,String> {
    store.get_connection().map_err(|_| "poll_store".into())
}

#[tauri::command]
#[specta::specta]
pub async fn poll_create(app: AppHandle, store: Meetings<'_>, svc: Service<'_>, runtime: Runtime<'_>, integration_id: String, poll: PollDefinition) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let view = polls::create(&svc,&store,&integration_id,poll.into_core())?;
    polls::updated(&app,&view.poll.id);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_get(store: Meetings<'_>, runtime: Runtime<'_>, id: String) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    polls::purge(&conn,polls::now())?;
    polls::get(&conn,&id)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_update(app: AppHandle, store: Meetings<'_>, svc: Service<'_>, runtime: Runtime<'_>, id: String, poll: PollDefinition) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    let view = polls::update_draft(&svc,&conn,&id,poll.into_core())?;
    polls::updated(&app,&id);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_rank(store: Meetings<'_>, svc: Service<'_>, runtime: Runtime<'_>, id: String) -> Result<Vec<besprechungsplanung::terminfindung::Rang>,String> {
    let _guard = runtime.operation.lock().await;
    runtime.rank(&svc,&conn(&store)?,&id)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_list(store: Meetings<'_>, runtime: Runtime<'_>) -> Result<Vec<PollDefinition>,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    polls::purge(&conn,polls::now())?;
    polls::list(&conn)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_availability(store: Meetings<'_>, svc: Service<'_>, runtime: Runtime<'_>, id: String, window: ScheduleWindow) -> Result<Vec<PersonAvailability>,String> {
    let _guard = runtime.operation.lock().await;
    runtime.availability(&svc,&store,&id,window).await
}

#[tauri::command]
#[specta::specta]
pub async fn poll_send(app: AppHandle, store: Meetings<'_>, svc: Service<'_>, runtime: Runtime<'_>, id: String) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    runtime.send(&app,Arc::clone(&svc),Arc::clone(&store),&id).await
}

#[tauri::command]
#[specta::specta]
pub async fn poll_answer_confirm(app: AppHandle, store: Meetings<'_>, runtime: Runtime<'_>, id: String, answer_id: i64) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    polls::confirm(&conn,&id,answer_id)?;
    polls::updated(&app,&id);
    polls::get(&conn,&id)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_answer_set(app: AppHandle, store: Meetings<'_>, runtime: Runtime<'_>, id: String, person: String, values: Vec<AntwortWert>) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    polls::set_answer(&conn,&id,&person,values,polls::now())?;
    polls::updated(&app,&id);
    polls::get(&conn,&id)
}

#[tauri::command]
#[specta::specta]
pub async fn poll_close(app: AppHandle, store: Meetings<'_>, runtime: Runtime<'_>, id: String, decided: bool) -> Result<PollView,String> {
    let _guard = runtime.operation.lock().await;
    let conn = conn(&store)?;
    polls::close(&conn,&id,decided,polls::now())?;
    runtime.clear_availability();
    polls::updated(&app,&id);
    polls::get(&conn,&id)
}
