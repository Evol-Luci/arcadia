//! Achievement Hub — RetroAchievements integration (v1.0).
//!
//! Arcadia reads the user's RetroAchievements progress through the official Web
//! API; it never awards or computes achievements itself (the emulator's
//! RetroAchievements support does that at runtime). Credentials are user-owned
//! (their RA username + personal Web API key) and the Hub stays dark until they
//! are configured — we ship no keys.
//!
//! A local game is *linked* to an RA game id (looked up by title, or set
//! manually), after which we cache the achievement set and the user's unlock
//! state so the UI works offline and we stay within RA's rate limits.

use crate::config::RetroAchievementsCredentials;
use crate::error::{EngineError, Result};
use crate::models::{Achievement, RaLink};
use crate::{new_id, now_rfc3339, Engine};
use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;

const API_BASE: &str = "https://retroachievements.org/API";
const BADGE_BASE: &str = "https://media.retroachievements.org/Badge";

/// A candidate RA game for the link picker.
#[derive(Debug, Clone, Serialize)]
pub struct RaGameCandidate {
    pub ra_game_id: i64,
    pub title: String,
}

/// Headline numbers for the signed-in RA user.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RaUserSummary {
    pub username: String,
    pub total_points: i64,
    pub rank: i64,
    pub recently_played_count: i64,
}

/// One row of the Achievement Hub overview: a linked game plus its cached
/// progress counts, so the hub can render the whole collection from one query.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct RaGameProgress {
    pub game_id: String,
    pub title: String,
    pub cover_art: Option<String>,
    pub ra_title: Option<String>,
    pub icon_url: Option<String>,
    pub total: i64,
    pub unlocked: i64,
    pub points_earned: i64,
    pub points_total: i64,
}

/// Map an Arcadia platform slug to a RetroAchievements console id. `None` for
/// platforms RA doesn't track (so the UI can hide linking for them).
fn ra_console_id(platform: &str) -> Option<i64> {
    Some(match platform {
        "genesis" => 1,
        "n64" => 2,
        "snes" => 3,
        "gb" => 4,
        "gba" => 5,
        "gbc" => 6,
        "nes" => 7,
        "pcengine" => 8,
        "segacd" => 9,
        "sega32x" => 10,
        "sms" => 11,
        "ps1" => 12,
        "lynx" => 13,
        "ngp" => 14,
        "gamegear" => 15,
        "gamecube" => 16,
        "jaguar" => 17,
        "nds" => 18,
        "ps2" => 21,
        "atari2600" => 25,
        "arcade" => 27,
        "virtualboy" => 28,
        "saturn" => 39,
        "dreamcast" => 40,
        "psp" => 41,
        "colecovision" => 44,
        "wonderswan" => 46,
        "atari7800" => 51,
        "msx" => 53,
        _ => return None,
    })
}

fn badge_url(badge_name: &str) -> Option<String> {
    let name = badge_name.trim();
    if name.is_empty() || name == "0" {
        return None;
    }
    Some(format!("{BADGE_BASE}/{name}.png"))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_default()
}

/// Parse `GetGameInfoAndUserProgress` into (game title, icon url, achievements).
/// Defensive: RA returns achievements as an object keyed by id, each with
/// `DateEarned` present only when unlocked. Pure, so it is unit-tested.
fn parse_game_progress(body: &str, game_id: &str) -> Result<(Option<String>, Option<String>, Vec<Achievement>)> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| EngineError::Invalid(format!("RA progress JSON: {e}")))?;
    let title = v.get("Title").and_then(|t| t.as_str()).map(String::from);
    let icon = v
        .get("ImageIcon")
        .and_then(|t| t.as_str())
        .map(|p| format!("https://media.retroachievements.org{p}"));

    let mut out = Vec::new();
    if let Some(map) = v.get("Achievements").and_then(|a| a.as_object()) {
        for (_key, ach) in map {
            let ra_id = ach.get("ID").and_then(value_as_i64).unwrap_or(0);
            if ra_id == 0 {
                continue;
            }
            let unlocked_at = ach
                .get("DateEarnedHardcore")
                .and_then(|d| d.as_str())
                .or_else(|| ach.get("DateEarned").and_then(|d| d.as_str()))
                .filter(|s| !s.is_empty())
                .map(String::from);
            out.push(Achievement {
                id: String::new(), // assigned at insert
                game_id: game_id.to_string(),
                ra_id,
                title: ach
                    .get("Title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)")
                    .to_string(),
                description: ach.get("Description").and_then(|d| d.as_str()).map(String::from),
                points: ach.get("Points").and_then(value_as_i64).unwrap_or(0),
                badge_url: ach.get("BadgeName").and_then(|b| b.as_str()).and_then(badge_url),
                unlocked: unlocked_at.is_some(),
                unlocked_at,
                display_order: ach
                    .get("DisplayOrder")
                    .and_then(value_as_i64)
                    .unwrap_or(0),
            });
        }
    }
    out.sort_by_key(|a| (a.display_order, a.ra_id));
    Ok((title, icon, out))
}

/// RA serialises numbers inconsistently (sometimes as JSON strings). Accept both.
fn value_as_i64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

impl Engine {
    fn ra_credentials(&self) -> Result<RetroAchievementsCredentials> {
        let creds = self.load_config().retroachievements;
        if !creds.is_configured() {
            return Err(EngineError::Invalid(
                "RetroAchievements credentials not configured".into(),
            ));
        }
        Ok(creds)
    }

    /// Whether RA credentials are present (so the UI can show/hide the Hub).
    pub fn retroachievements_configured(&self) -> bool {
        self.load_config().retroachievements.is_configured()
    }

    /// Look up RA games whose title matches `query` for a game's platform, so the
    /// user can link without hunting for a numeric id. Empty for unsupported
    /// platforms.
    pub async fn search_ra_games(&self, platform: &str, query: &str) -> Result<Vec<RaGameCandidate>> {
        let creds = self.ra_credentials()?;
        let Some(console) = ra_console_id(platform) else {
            return Ok(Vec::new());
        };
        let url = format!(
            "{API_BASE}/API_GetGameList.php?z={u}&y={k}&i={console}&f=1",
            u = urlencode(&creds.username),
            k = urlencode(&creds.api_key),
        );
        let body = client()
            .get(&url)
            .send()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA request failed: {e}")))?
            .text()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA response: {e}")))?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| EngineError::Invalid(format!("RA game list JSON: {e}")))?;
        let needle = query.trim().to_lowercase();
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for g in arr {
                let title = g.get("Title").and_then(|t| t.as_str()).unwrap_or("");
                if needle.is_empty() || title.to_lowercase().contains(&needle) {
                    if let Some(id) = g.get("ID").and_then(value_as_i64) {
                        out.push(RaGameCandidate { ra_game_id: id, title: title.to_string() });
                    }
                }
                if out.len() >= 50 {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Link a local game to an RA game id, then fetch its achievements.
    pub async fn link_ra_game(&self, game_id: &str, ra_game_id: i64) -> Result<usize> {
        sqlx::query(
            "INSERT INTO ra_links (game_id, ra_game_id, linked_at) VALUES (?,?,?)
             ON CONFLICT(game_id) DO UPDATE SET ra_game_id = excluded.ra_game_id, linked_at = excluded.linked_at",
        )
        .bind(game_id)
        .bind(ra_game_id)
        .bind(now_rfc3339())
        .execute(&self.pool)
        .await?;
        self.refresh_achievements(game_id).await
    }

    /// Remove a game's RA link and cached achievements.
    pub async fn unlink_ra_game(&self, game_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM achievements WHERE game_id = ?")
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM ra_links WHERE game_id = ?")
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_ra_link(&self, game_id: &str) -> Result<Option<RaLink>> {
        Ok(sqlx::query_as::<_, RaLink>("SELECT * FROM ra_links WHERE game_id = ?")
            .bind(game_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Re-fetch achievements + unlock state for a linked game from RA and cache
    /// them. Returns the number of achievements stored.
    pub async fn refresh_achievements(&self, game_id: &str) -> Result<usize> {
        let creds = self.ra_credentials()?;
        let link = self
            .get_ra_link(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("RA link for game {game_id}")))?;

        let url = format!(
            "{API_BASE}/API_GetGameInfoAndUserProgress.php?z={u}&y={k}&u={u}&g={g}",
            u = urlencode(&creds.username),
            k = urlencode(&creds.api_key),
            g = link.ra_game_id,
        );
        let body = client()
            .get(&url)
            .send()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA request failed: {e}")))?
            .text()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA response: {e}")))?;
        let (title, icon, achievements) = parse_game_progress(&body, game_id)?;

        // Update the link's display fields.
        sqlx::query("UPDATE ra_links SET title = ?, icon_url = ? WHERE game_id = ?")
            .bind(&title)
            .bind(&icon)
            .bind(game_id)
            .execute(&self.pool)
            .await?;

        // Replace cached achievements wholesale (RA is the source of truth).
        sqlx::query("DELETE FROM achievements WHERE game_id = ?")
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        for ach in &achievements {
            sqlx::query(
                "INSERT INTO achievements
                   (id, game_id, ra_id, title, description, points, badge_url, unlocked, unlocked_at, display_order)
                 VALUES (?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(new_id())
            .bind(game_id)
            .bind(ach.ra_id)
            .bind(&ach.title)
            .bind(&ach.description)
            .bind(ach.points)
            .bind(&ach.badge_url)
            .bind(ach.unlocked)
            .bind(&ach.unlocked_at)
            .bind(ach.display_order)
            .execute(&self.pool)
            .await?;
        }
        Ok(achievements.len())
    }

    pub async fn list_achievements(&self, game_id: &str) -> Result<Vec<Achievement>> {
        Ok(sqlx::query_as::<_, Achievement>(
            "SELECT * FROM achievements WHERE game_id = ? ORDER BY unlocked DESC, display_order, ra_id",
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Overview of every RA-linked game for this profile, with cached progress
    /// counts. Drives the Achievement Hub list without N round-trips to RA.
    pub async fn list_ra_linked_games(&self, profile_id: &str) -> Result<Vec<RaGameProgress>> {
        Ok(sqlx::query_as::<_, RaGameProgress>(
            "SELECT g.id AS game_id,
                    g.title AS title,
                    g.cover_art AS cover_art,
                    l.title AS ra_title,
                    l.icon_url AS icon_url,
                    COUNT(a.id) AS total,
                    COALESCE(SUM(a.unlocked), 0) AS unlocked,
                    COALESCE(SUM(CASE WHEN a.unlocked = 1 THEN a.points ELSE 0 END), 0) AS points_earned,
                    COALESCE(SUM(a.points), 0) AS points_total
             FROM ra_links l
             JOIN games g ON g.id = l.game_id
             LEFT JOIN achievements a ON a.game_id = l.game_id
             WHERE g.profile_id = ?
             GROUP BY g.id
             ORDER BY g.title",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Headline summary for the signed-in RA user.
    pub async fn ra_user_summary(&self) -> Result<RaUserSummary> {
        let creds = self.ra_credentials()?;
        let url = format!(
            "{API_BASE}/API_GetUserSummary.php?z={u}&y={k}&u={u}&g=5&a=5",
            u = urlencode(&creds.username),
            k = urlencode(&creds.api_key),
        );
        let body = client()
            .get(&url)
            .send()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA request failed: {e}")))?
            .text()
            .await
            .map_err(|e| EngineError::Invalid(format!("RA response: {e}")))?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| EngineError::Invalid(format!("RA summary JSON: {e}")))?;
        Ok(RaUserSummary {
            username: creds.username,
            total_points: v.get("TotalPoints").and_then(value_as_i64).unwrap_or(0),
            rank: v.get("Rank").and_then(value_as_i64).unwrap_or(0),
            recently_played_count: v
                .get("RecentlyPlayed")
                .and_then(|r| r.as_array())
                .map(|a| a.len() as i64)
                .unwrap_or(0),
        })
    }
}

/// Minimal percent-encoder for query values (RA keys are alnum, usernames may
/// contain spaces/specials). Mirrors the helper in `metadata.rs`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_with_mixed_unlock_state() {
        let body = r#"{
            "Title": "Sonic the Hedgehog",
            "ImageIcon": "/Images/001.png",
            "Achievements": {
                "1": {"ID": 1, "Title": "Ring King", "Description": "Get 100 rings",
                       "Points": 5, "BadgeName": "12345", "DisplayOrder": 0,
                       "DateEarned": "2024-01-02 10:00:00"},
                "2": {"ID": "2", "Title": "Speedrun", "Description": "Beat zone 1 fast",
                       "Points": "10", "BadgeName": "0", "DisplayOrder": 1}
            }
        }"#;
        let (title, icon, achs) = parse_game_progress(body, "g1").unwrap();
        assert_eq!(title.as_deref(), Some("Sonic the Hedgehog"));
        assert!(icon.unwrap().ends_with("/Images/001.png"));
        assert_eq!(achs.len(), 2);
        // Sorted by display order.
        assert_eq!(achs[0].ra_id, 1);
        assert!(achs[0].unlocked);
        assert_eq!(achs[0].badge_url.as_deref(), Some("https://media.retroachievements.org/Badge/12345.png"));
        // String-encoded points/id parsed; "0" badge dropped; not unlocked.
        assert_eq!(achs[1].ra_id, 2);
        assert_eq!(achs[1].points, 10);
        assert!(!achs[1].unlocked);
        assert!(achs[1].badge_url.is_none());
    }

    #[test]
    fn console_mapping_covers_ra_supported_platforms() {
        for p in [
            "nes", "snes", "n64", "gb", "gbc", "gba", "nds", "genesis", "ps1", "ps2", "psp",
            "pcengine", "segacd", "sega32x", "sms", "lynx", "ngp", "gamegear", "jaguar",
            "atari2600", "arcade", "virtualboy", "saturn", "dreamcast", "colecovision",
            "wonderswan", "atari7800", "msx",
        ] {
            assert!(ra_console_id(p).is_some(), "no RA console id for {p}");
        }
        // RA doesn't track these, so linking stays hidden.
        assert!(ra_console_id("ps3").is_none());
        assert!(ra_console_id("switch").is_none());
    }
}
