use termnest_core::snippets::Snippet;
use termnest_core::CoreError;

use super::AppStateRef;
use crate::events::SNIPPETS_CHANGED_EVENT;

#[tauri::command]
pub fn snippets_list(state: AppStateRef<'_>) -> Vec<Snippet> {
    state.snippets.list()
}

#[tauri::command]
pub fn snippets_upsert(state: AppStateRef<'_>, snippet: Snippet) -> Result<Snippet, CoreError> {
    let saved = state.snippets.upsert(snippet)?;
    state.emit(SNIPPETS_CHANGED_EVENT, state.snippets.list());
    Ok(saved)
}

#[tauri::command]
pub fn snippets_delete(state: AppStateRef<'_>, id: String) -> Result<(), CoreError> {
    state.snippets.delete(&id)?;
    state.emit(SNIPPETS_CHANGED_EVENT, state.snippets.list());
    Ok(())
}
