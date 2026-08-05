//! Petdex mascot HTTP API — lets the desktop app (and any web surface)
//! render the active pet: config poll, installed-pet list, and raw
//! spritesheet bytes. Mirrors hermes' desktop pet overlay data flow
//! (`display.pet.*` config + `<home>/pets/<slug>/spritesheet.*`).

use axum::{
    extract::Path,
    http::header,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Atlas layout shared by every renderer (hermes Codex 9-row sheets).
fn atlas_layout() -> serde_json::Value {
    let rows: Vec<serde_json::Value> = crate::pets_atlas::ROW_SPECS
        .iter()
        .map(|(state, row, frames)| json!({"state": state, "row": row, "frames": frames}))
        .collect();
    json!({
        "frame_w": crate::pets_atlas::CELL_WIDTH,
        "frame_h": crate::pets_atlas::CELL_HEIGHT,
        "columns": crate::pets_atlas::COLUMNS,
        "rows": rows,
    })
}

/// `GET /api/pets/config` — active `display.pet` settings + atlas layout.
pub async fn config() -> Response {
    let cfg = crate::config::UlncLawConfig::load(None).unwrap_or_default();
    let pet = &cfg.display.pet;
    Json(json!({
        "object": "ulnclaw.pet.config",
        "enabled": pet.enabled,
        "slug": pet.slug,
        "scale": pet.scale,
        "render_mode": pet.render_mode,
        "atlas": atlas_layout(),
    }))
    .into_response()
}

/// `GET /api/pets` — installed pets.
pub async fn list() -> Response {
    let home = crate::config::ulnclaw_home();
    let pets = crate::pets::installed_pets(&home);
    let active = crate::config::UlncLawConfig::load(None)
        .unwrap_or_default()
        .display
        .pet
        .slug;
    let data: Vec<serde_json::Value> = pets
        .iter()
        .map(|p| {
            json!({
                "slug": p.slug,
                "display_name": p.display_name,
                "description": p.description,
                "created_by": p.created_by,
                "active": active.as_deref() == Some(p.slug.as_str()),
                "spritesheet": format!("/api/pets/{}/spritesheet", p.slug),
            })
        })
        .collect();
    Json(json!({"object": "ulnclaw.pet.list", "data": data})).into_response()
}

/// `GET /api/pets/:slug/spritesheet` — raw sheet bytes (webp or png).
pub async fn spritesheet(Path(slug): Path<String>) -> Response {
    let home = crate::config::ulnclaw_home();
    let Some(pet) = crate::pets::load_pet(&home, &slug) else {
        return super::not_found(&format!("pet {slug} not installed"));
    };
    if !pet.exists() {
        return super::not_found(&format!("pet {slug} has no spritesheet"));
    }
    let bytes = match std::fs::read(&pet.spritesheet) {
        Ok(b) => b,
        Err(e) => return super::server_error(&e.to_string()),
    };
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else {
        "image/webp"
    };
    (
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "max-age=300")],
        bytes,
    )
        .into_response()
}
