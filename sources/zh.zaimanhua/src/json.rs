use aidoku::{
    Result,
    alloc::{String, Vec, format, vec, string::ToString},
    Chapter, Manga, MangaPageResult, MangaStatus, ContentRating,
};

/// Parse manga list from serde_json Value array
/// Access control is handled by server based on authentication status
pub fn parse_manga_list(data: &serde_json::Value) -> Result<MangaPageResult> {
    let mut entries = Vec::new();
    
    if let Some(arr) = data.as_array() {
        for item in arr {
            // Get manga key: prefer 'id' field, fallback to 'comic_id'
            let key = item.get("id")
                .or_else(|| item.get("comic_id"))
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_default();
            
            if key.is_empty() || key == "0" {
                continue;
            }

            let title = item.get("title")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into();

            let cover = item.get("cover")
                .and_then(|v| v.as_str())
                .map(String::from);

            let author_str = item.get("authors")
                .and_then(|v| v.as_str())
                .map(String::from);
            
            let authors = author_str.map(|a| vec![a]);

            let status_str = item.get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let status = parse_status(status_str);

            entries.push(Manga {
                key,
                title,
                cover,
                authors,
                status,
                content_rating: ContentRating::Safe,
                ..Default::default()
            });
        }
    }
    
    let has_next_page = !entries.is_empty();
    Ok(MangaPageResult { entries, has_next_page })
}

/// Parse rank list from serde_json Value array (rank API returns different field names)
pub fn parse_rank_list(data: &serde_json::Value) -> Result<MangaPageResult> {
    let mut entries = Vec::new();
    
    if let Some(arr) = data.as_array() {
        for item in arr {
            let key = item.get("comic_id")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_default();
            
            if key.is_empty() || key == "0" {
                continue;
            }

            let title = item.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into();

            let cover = item.get("cover")
                .and_then(|v| v.as_str())
                .map(String::from);

            let author_str = item.get("authors")
                .and_then(|v| v.as_str())
                .map(String::from);
            
            let authors = author_str.map(|a| vec![a]);

            entries.push(Manga {
                key,
                title,
                cover,
                authors,
                content_rating: ContentRating::Safe,
                ..Default::default()
            });
        }
    }
    
    let has_next_page = !entries.is_empty();
    Ok(MangaPageResult { entries, has_next_page })
}

/// Parse subscribe list from serde_json Value
/// API response structure: { "subList": [...] }
pub fn parse_subscribe_list(data: &serde_json::Value) -> Result<MangaPageResult> {
    let mut entries = Vec::new();
    
    if let Some(arr) = data.get("subList").and_then(|v| v.as_array()) {
        for item in arr {
            let key = item.get("id")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_default();
            
            if key.is_empty() || key == "0" {
                continue;
            }

            let title = item.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into();

            let cover = item.get("cover")
                .and_then(|v| v.as_str())
                .map(String::from);

            let author_str = item.get("authors")
                .and_then(|v| v.as_str())
                .map(String::from);
            
            let authors = author_str.map(|a| vec![a]);

            let status_str = item.get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let status = parse_status(status_str);

            entries.push(Manga {
                key,
                title,
                cover,
                authors,
                status,
                content_rating: ContentRating::Safe,
                ..Default::default()
            });
        }
    }
    
    // 订阅列表通常一次返回全部，但如果数量等于 size 可能有下一页
    let has_next_page = entries.len() >= 50;
    Ok(MangaPageResult { entries, has_next_page })
}

/// Parse manga details from serde_json Value
pub fn parse_manga_details(manga_data: &serde_json::Value, key: String) -> Result<Manga> {
    let title = manga_data.get("title")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .into();

    let cover = manga_data.get("cover")
        .and_then(|v| v.as_str())
        .map(String::from);

    let description = manga_data.get("description")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Authors is array of {tag_name: "..."}
    let authors: Option<Vec<String>> = manga_data.get("authors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
               .filter_map(|a| a.get("tag_name"))
               .filter_map(|v| v.as_str())
               .map(String::from)
               .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());

    // Types is array of {tag_name: "..."}
    let tags: Option<Vec<String>> = manga_data.get("types")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
               .filter_map(|t| t.get("tag_name"))
               .filter_map(|v| v.as_str())
               .map(String::from)
               .collect()
        })
        .filter(|v: &Vec<String>| !v.is_empty());

    // Status from array
    let mut status = MangaStatus::Unknown;
    if let Some(status_arr) = manga_data.get("status").and_then(|v| v.as_array()) {
        if let Some(first) = status_arr.first() {
            if let Some(tag_name) = first.get("tag_name").and_then(|v| v.as_str()) {
                status = parse_status(tag_name);
            }
        }
    }

    Ok(Manga {
        key,
        title,
        cover,
        authors,
        description,
        tags,
        status,
        content_rating: ContentRating::Safe,
        ..Default::default()
    })
}

/// Parse chapter list from serde_json Value
/// API returns chapters in newest-first order (208话 → 1话)
/// We keep this order for display but assign chapter_number so first chapter has lowest number
pub fn parse_chapters(manga_data: &serde_json::Value, manga_id: &str) -> Result<Vec<Chapter>> {
    // Single pass: collect all chapter data first, then assign numbers
    let mut raw_chapters: Vec<(String, Option<String>, String, Option<i64>)> = Vec::new();
    
    if let Some(chapters_arr) = manga_data.get("chapters").and_then(|v| v.as_array()) {
        for group in chapters_arr {
            let group_title: String = group.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .into();
            
            if let Some(group_data) = group.get("data").and_then(|v| v.as_array()) {
                for chapter in group_data {
                    let chapter_id = chapter.get("chapter_id")
                        .and_then(|v| v.as_i64())
                        .map(|n| n.to_string())
                        .unwrap_or_default();
                    
                    let chapter_title = chapter.get("chapter_title")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    
                    let updatetime = chapter.get("updatetime")
                        .and_then(|v| v.as_i64());
                    
                    raw_chapters.push((chapter_id, chapter_title, group_title.clone(), updatetime));
                }
            }
        }
    }
    
    // Assign chapter numbers: newest (first in API) gets highest number
    let total = raw_chapters.len() as f32;
    let chapters = raw_chapters.into_iter()
        .enumerate()
        .map(|(idx, (chapter_id, chapter_title, group_title, updatetime))| {
            Chapter {
                key: format!("{}/{}", manga_id, chapter_id),
                title: chapter_title,
                chapter_number: Some(total - idx as f32),
                scanlators: Some(vec![group_title]),
                date_uploaded: updatetime,
                ..Default::default()
            }
        })
        .collect();
    
    Ok(chapters)
}

fn parse_status(status_str: &str) -> MangaStatus {
    if status_str.contains("连载") {
        MangaStatus::Ongoing
    } else if status_str.contains("完结") {
        MangaStatus::Completed
    } else {
        MangaStatus::Unknown
    }
}
