use crate::{get_api_request, net, V4_API_URL};
use aidoku::{
    Chapter, HomeComponent, HomeLayout, HomePartialResult, 
    Listing, ListingKind, Manga, MangaWithChapter, Result,
    alloc::{String, Vec, format, vec, string::ToString},
    imports::{
        net::{Request, RequestError, Response},
        std::send_partial_result,
    },
};

/// Build the home page layout with comprehensive components
pub fn get_home_layout() -> Result<HomeLayout> {
    // 1. Send skeleton layout first for progressive loading
    // Layout: Banner → 精品推荐 → 人气推荐 → 最近更新 → 少年/少女/男青/女青
    send_partial_result(&HomePartialResult::Layout(HomeLayout {
        components: vec![
            HomeComponent {
                title: None,
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_image_scroller(),
            },
            HomeComponent {
                title: Some("精品推荐".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_big_scroller(),
            },
            HomeComponent {
                title: Some("人气推荐".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_manga_list(),
            },
            HomeComponent {
                title: Some("最近更新".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_manga_chapter_list(),
            },
            HomeComponent {
                title: Some("少年漫画".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_scroller(),
            },
            HomeComponent {
                title: Some("少女漫画".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_scroller(),
            },
            HomeComponent {
                title: Some("男青漫画".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_scroller(),
            },
            HomeComponent {
                title: Some("女青漫画".into()),
                subtitle: None,
                value: aidoku::HomeComponentValue::empty_scroller(),
            },
        ],
    }));

    // 2. Concurrent API requests
    let recommend_url = format!("{}/comic/recommend/list", V4_API_URL);
    let latest_url = format!("{}/comic/filter/list?sortType=1&size=20&page=1", V4_API_URL);
    // Use Rank API 月榜 - 1 page = 10 items
    let rank_url = format!("{}/comic/rank/list?rank_type=0&by_time=2&page=1", V4_API_URL);
    // Audience categories
    let shounen_url = format!("{}/comic/filter/list?cate=3262&size=20&page=1", V4_API_URL);
    let shoujo_url = format!("{}/comic/filter/list?cate=3263&size=20&page=1", V4_API_URL);
    let seinen_url = format!("{}/comic/filter/list?cate=3264&size=20&page=1", V4_API_URL);
    let josei_url = format!("{}/comic/filter/list?cate=13626&size=20&page=1", V4_API_URL);
    // 漫画情报 HTML page for Banner
    let manga_news_url = "https://news.zaimanhua.com/manhuaqingbao";

    // 8 requests
    let requests = [
        net::get_request(&recommend_url)?,      // 0: recommend
        get_api_request(&latest_url)?,          // 1: latest
        get_api_request(&rank_url)?,            // 2: rank
        get_api_request(&shounen_url)?,         // 3: 少年漫画
        get_api_request(&shoujo_url)?,          // 4: 少女漫画
        get_api_request(&seinen_url)?,          // 5: 男青漫画
        get_api_request(&josei_url)?,           // 6: 女青漫画
        net::get_request(manga_news_url)?,      // 7: 漫画情报 HTML
    ];

    let mut responses: [core::result::Result<Response, RequestError>; 8] = 
        Request::send_all(requests)
            .try_into()
            .map_err(|_| aidoku::error!("Failed to convert responses"))?;

    // 3. Parse responses
    let mut components = Vec::new();
    
    // Variables for parsed data
    let mut banner_links: Vec<aidoku::Link> = Vec::new();
    let mut big_scroller_manga: Vec<Manga> = Vec::new();

    // Parse 漫画情报 HTML (index 7) - for Banner
    if let Ok(ref mut resp) = responses[7] {
        if let Ok(html) = resp.get_string() {
            banner_links = parse_manga_news_html(&html);
        }
    }

    // Parse recommend/list response (index 0) - for BigScroller only
    if let Ok(ref mut resp) = responses[0] {
        if let Ok(data) = resp.get_json::<serde_json::Value>() {
            if let Some(categories) = data.as_array() {
                for cat in categories {
                    let cat_id = cat.get("category_id").and_then(|v| v.as_i64()).unwrap_or(0);
                    // category_id=109 is "大图推荐" - for BigScroller
                    if cat_id == 109 {
                        big_scroller_manga = fetch_banner_manga_details(cat);
                    }
                }
            }
        }
    }

    // Parse filter/list latest response (index 1) - for 最近更新
    let mut latest_entries: Vec<MangaWithChapter> = Vec::new();
    if let Ok(ref mut resp) = responses[1] {
        if let Ok(data) = resp.get_json::<serde_json::Value>() {
            if let Some(list) = data.get("data")
                .and_then(|d| d.get("comicList"))
                .and_then(|v| v.as_array()) {
                latest_entries = list.iter()
                    .filter_map(|item| parse_manga_with_chapter(item))
                    .collect();
            }
        }
    }

    // Helper to parse rank page - simplified, only heat in description
    fn parse_rank_page(resp: &mut Response) -> Vec<Manga> {
        if let Ok(data) = resp.get_json::<serde_json::Value>() {
            if let Some(list) = data.get("data").and_then(|v| v.as_array()) {
                return list.iter()
                    .filter_map(|item| {
                        let id = item.get("comic_id")?.as_i64()?.to_string();
                        let title = item.get("title")?.as_str()?.into();
                        let cover = item.get("cover").and_then(|v| v.as_str()).map(String::from);
                        
                        // Parse authors (shown as subtitle by Aidoku)
                        let author_str = item.get("authors").and_then(|a| a.as_str()).unwrap_or("");
                        let authors = if author_str.is_empty() { 
                            None 
                        } else { 
                            Some(vec![author_str.to_string()]) 
                        };
                        
                        // Parse 热度 only (no tags)
                        let num = item.get("num").and_then(|n| n.as_i64()).unwrap_or(0);
                        let description = if num >= 10000 {
                            Some(format!("热度 {:.1}万", num as f64 / 10000.0))
                        } else if num > 0 {
                            Some(format!("热度 {}", num))
                        } else {
                            None
                        };
                        
                        Some(Manga {
                            key: id,
                            title,
                            cover,
                            authors,
                            description,
                            ..Default::default()
                        })
                    })
                    .collect();
            }
        }
        Vec::new()
    }
    
    // Parse人气推荐 rank data (1 page = 10 items)
    let mut hot_entries: Vec<Manga> = Vec::new();
    if let Ok(ref mut resp) = responses[2] { hot_entries.extend(parse_rank_page(resp)); }
    
    // Component 1: ImageScroller - Banner (手动滚动)
    components.push(HomeComponent {
        title: None,
        subtitle: None,
        value: aidoku::HomeComponentValue::ImageScroller {
            links: banner_links,
            auto_scroll_interval: Some(5.0),  // Auto scroll every 5 seconds
            width: Some(375),
            height: Some(240),
        },
    });

    // Component 2: BigScroller - 精品推荐 (only editorial picks)
    // BigScroller will display tags as buttons at bottom
    let premium_manga: Vec<Manga> = big_scroller_manga;
    
    components.push(HomeComponent {
        title: Some("精品推荐".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::BigScroller {
            entries: premium_manga,
            auto_scroll_interval: Some(8.0),
        },
    });

    // Component 3: MangaList - 人气推荐 (already parsed above)
    
    components.push(HomeComponent {
        title: Some("人气推荐".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::MangaList {
            ranking: true,
            page_size: Some(2),
            entries: hot_entries.into_iter().map(|manga| {
                // Only show author in subtitle
                let subtitle = manga.authors.as_ref()
                    .filter(|a| !a.is_empty())
                    .map(|a| a.join(", "));
                
                aidoku::Link {
                    title: manga.title.clone(),
                    subtitle,
                    image_url: manga.cover.clone(),
                    value: Some(aidoku::LinkValue::Manga(manga)),
                }
            }).collect(),
            listing: Some(Listing {
                id: "rank-monthly".into(),
                name: "人气推荐".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    // Component 4: MangaChapterList - 最近更新
    components.push(HomeComponent {
        title: Some("最近更新".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::MangaChapterList {
            page_size: Some(4),
            entries: latest_entries,
            listing: Some(Listing {
                id: "latest".into(),
                name: "更新".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    // Helper to parse audience category scroller with author info
    fn parse_audience_scroller(resp: &mut Response) -> Vec<aidoku::Link> {
        if let Ok(data) = resp.get_json::<serde_json::Value>() {
            if let Some(list) = data.get("data")
                .and_then(|d| d.get("comicList"))
                .and_then(|v| v.as_array()) {
                return list.iter()
                    .filter_map(|item| {
                        let id = item.get("id")?.as_i64()?.to_string();
                        let title = item.get("name")?.as_str()?.into();
                        let cover = item.get("cover").and_then(|v| v.as_str()).map(String::from);
                        let author = item.get("authors").and_then(|a| a.as_str()).map(String::from);
                        
                        Some(aidoku::Link {
                            title,
                            subtitle: author,
                            image_url: cover,
                            value: Some(aidoku::LinkValue::Manga(Manga {
                                key: id,
                                ..Default::default()
                            })),
                        })
                    })
                    .collect();
            }
        }
        Vec::new()
    }

    // Component 5: Scroller - 少年漫画
    let shounen_links = if let Ok(ref mut resp) = responses[3] {
        parse_audience_scroller(resp)
    } else { Vec::new() };
    components.push(HomeComponent {
        title: Some("少年漫画".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::Scroller {
            entries: shounen_links,
            listing: Some(Listing {
                id: "shounen".into(),
                name: "少年漫画".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    // Component 6: Scroller - 少女漫画
    let shoujo_links = if let Ok(ref mut resp) = responses[4] {
        parse_audience_scroller(resp)
    } else { Vec::new() };
    components.push(HomeComponent {
        title: Some("少女漫画".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::Scroller {
            entries: shoujo_links,
            listing: Some(Listing {
                id: "shoujo".into(),
                name: "少女漫画".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    // Component 7: Scroller - 男青漫画
    let seinen_links = if let Ok(ref mut resp) = responses[5] {
        parse_audience_scroller(resp)
    } else { Vec::new() };
    components.push(HomeComponent {
        title: Some("男青漫画".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::Scroller {
            entries: seinen_links,
            listing: Some(Listing {
                id: "seinen".into(),
                name: "男青漫画".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    // Component 8: Scroller - 女青漫画
    let josei_links = if let Ok(ref mut resp) = responses[6] {
        parse_audience_scroller(resp)
    } else { Vec::new() };
    components.push(HomeComponent {
        title: Some("女青漫画".into()),
        subtitle: None,
        value: aidoku::HomeComponentValue::Scroller {
            entries: josei_links,
            listing: Some(Listing {
                id: "josei".into(),
                name: "女青漫画".into(),
                kind: ListingKind::default(),
            }),
        },
    });

    Ok(HomeLayout { components })
}

/// Parse manga news HTML page to extract article images and links
/// HTML structure: .briefnews_con_li contains .dec_img img (image) and h3 a (link)
fn parse_manga_news_html(html: &str) -> Vec<aidoku::Link> {
    let mut links = Vec::new();
    let mut seen_ids: Vec<String> = Vec::new();
    
    // Split by image markers and extract pairs
    for (i, part) in html.split("images.zaimanhua.com/news/article/").enumerate() {
        if i == 0 || links.len() >= 5 {
            continue;
        }
        
        // Extract article ID from image path (first segment after split)
        let article_id: String = part.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        
        if article_id.is_empty() || seen_ids.contains(&article_id) {
            continue;
        }
        seen_ids.push(article_id.clone());
        
        // Extract full image URL (find the end quote)
        let img_url_part: String = part.chars()
            .take_while(|c| *c != '"' && *c != '\'' && *c != ' ')
            .collect();
        
        let image_url = format!("https://images.zaimanhua.com/news/article/{}", img_url_part);
        let news_url = format!("https://news.zaimanhua.com/article/{}.html", article_id);
        
        links.push(aidoku::Link {
            title: String::new(),
            subtitle: None,
            image_url: Some(image_url),
            value: Some(aidoku::LinkValue::Url(news_url)),
        });
    }
    
    links
}


/// Fetch full manga details for BigScroller from banner entries (type=1 only)
/// Uses Request::send_all for parallel detail API requests
fn fetch_banner_manga_details(category: &serde_json::Value) -> Vec<Manga> {
    // Step 1: Collect manga IDs and banner text (both title and sub_title)
    let mut banner_data: Vec<(String, String, String)> = Vec::new(); // (manga_id, title, sub_title)
    
    if let Some(data) = category.get("data").and_then(|v| v.as_array()) {
        for item in data.iter() {
            let item_type = item.get("type").and_then(|v| v.as_i64()).unwrap_or(0);
            if item_type != 1 {
                continue;
            }
            
            let obj_id = item.get("obj_id")
                .and_then(|v| v.as_i64())
                .map(|n| n.to_string())
                .unwrap_or_default();
            
            if obj_id.is_empty() || obj_id == "0" {
                continue;
            }
            
            // Get both title and sub_title from banner data
            let banner_title = item.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            
            let banner_sub = item.get("sub_title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            
            banner_data.push((obj_id, banner_title, banner_sub));
        }
    }
    
    if banner_data.is_empty() {
        return Vec::new();
    }
    
    // Step 2: Build parallel requests for all detail APIs
    let requests: Vec<_> = banner_data.iter()
        .filter_map(|(manga_id, _, _)| {
            let url = format!("{}/comic/detail/{}", crate::V4_API_URL, manga_id);
            crate::net::get_request(&url).ok()
        })
        .collect();
    
    if requests.is_empty() {
        return Vec::new();
    }
    
    // Step 3: Send all requests in parallel
    let responses = Request::send_all(requests);
    
    // Step 4: Parse responses and build manga entries
    let mut entries = Vec::new();
    for (idx, resp_result) in responses.into_iter().enumerate() {
        if idx >= banner_data.len() { break; }
        
        let (manga_id, banner_title, _) = &banner_data[idx];
        
        if let Ok(mut resp) = resp_result {
            if let Ok(data) = resp.get_json::<serde_json::Value>() {
                if let Some(manga_data) = data.get("data").and_then(|d| d.get("data")) {
                    let title: String = manga_data.get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .into();
                    
                    let cover = manga_data.get("cover")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    
                    // Parse authors
                    let authors = manga_data.get("authors")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| t.get("tag_name").and_then(|n| n.as_str()))
                                .map(String::from)
                                .collect()
                        });
                    
                    // Parse tags from "types" array
                    let tags = manga_data.get("types")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| t.get("tag_name").and_then(|n| n.as_str()))
                                .map(String::from)
                                .collect()
                        });
                    
                    // Always use banner_title as editor note
                    let description = if !banner_title.is_empty() {
                        Some(banner_title.clone())
                    } else {
                        None
                    };
                    
                    entries.push(Manga {
                        key: manga_id.clone(),
                        title,
                        cover,
                        description,
                        authors,
                        tags,
                        ..Default::default()
                    });
                }
            }
        }
    }
    
    entries
}



/// Parse filter list item as MangaWithChapter for latest updates
fn parse_manga_with_chapter(item: &serde_json::Value) -> Option<MangaWithChapter> {
    let id = item.get("id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())?;
    
    let title = item.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .into();

    let cover = item.get("cover")
        .and_then(|v| v.as_str())
        .map(String::from);

    let chapter_name = item.get("last_update_chapter_name")
        .and_then(|v| v.as_str())
        .map(String::from);

    let chapter_id = item.get("last_update_chapter_id")
        .and_then(|v| v.as_i64())
        .map(|n| n.to_string())
        .unwrap_or_default();

    let updatetime = item.get("last_updatetime")
        .and_then(|v| v.as_i64());

    Some(MangaWithChapter {
        manga: Manga {
            key: id.clone(),
            title,
            cover,
            ..Default::default()
        },
        chapter: Chapter {
            key: format!("{}/{}", id, chapter_id),
            title: chapter_name,
            date_uploaded: updatetime,
            ..Default::default()
        },
    })
}
