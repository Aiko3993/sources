#![no_std]

use aidoku::{
    BasicLoginHandler, Chapter, DeepLinkHandler, DeepLinkResult, DynamicSettings, FilterValue,
    GroupSetting, Home, HomeLayout, ImageRequestProvider, Listing, ListingProvider, LoginMethod, LoginSetting, Manga, MangaPageResult, 
    Page, PageContent, PageContext, Result, Setting, Source, NotificationHandler, ToggleSetting,
    alloc::{String, Vec, format, string::ToString, vec},
    helpers::uri::encode_uri_component,
    imports::net::Request,
    prelude::*,
};

mod home;
mod json;
mod net;
mod settings;

pub const BASE_URL: &str = "https://www.zaimanhua.com/";
const V4_API_URL: &str = "https://v4api.zaimanhua.com/app/v1";


/// Helper function to create a GET request with optional authentication
/// Authentication is only added if:
/// 1. User is logged in (has token)
/// 2. Enhanced mode is enabled (user explicitly turned it on)
fn get_api_request(url: &str) -> Result<Request> {
    if let Some(token) = settings::get_token() {
        // Only use auth if enhanced mode is enabled
        if settings::get_enhanced_mode() {
            net::auth_request(url, &token)
        } else {
            net::get_request(url)
        }
    } else {
        net::get_request(url)
    }
}

// === Search Helper Functions ===

/// Search manga by keyword
fn search_by_keyword(keyword: &str, page: i32) -> Result<MangaPageResult> {
    let encoded = encode_uri_component(keyword);
    let url = format!(
        "{}/search/index?keyword={}&source=0&page={}&size=20",
        V4_API_URL, encoded, page
    );

    let mut response = get_api_request(&url)?.send()?;
    let json_val: serde_json::Value = response.get_json()?;

    let list = json_val
        .get("data")
        .and_then(|d| d.get("list"))
        .ok_or_else(|| error!("Missing data.list"))?;
    
    let total = json_val
        .get("data")
        .and_then(|d| d.get("total"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;
    
    let mut result = json::parse_manga_list(list)?;
    result.has_next_page = (page * 20) < total;
    Ok(result)
}

/// Browse manga with filters (including optional rank mode)
fn browse_with_filters(filters: &[FilterValue], page: i32) -> Result<MangaPageResult> {
    // Default values (API defaults)
    let mut sort_type = "1".to_string();  // 1=更新时间, 2=热门人气
    let mut zone = "0".to_string();       // 0=全部
    let mut status = "0".to_string();     // 0=全部
    let mut cate = "0".to_string();       // 0=全部
    let mut theme = "0".to_string();      // 0=全部
    let mut rank_mode = "0".to_string();  // 0=不使用, 1-4=日/周/月/总榜
    
    for filter in filters {
        if let FilterValue::Select { id, value } = filter {
            match id.as_str() {
                "排序" => sort_type = value.clone(),
                "地区" => zone = value.clone(),
                "状态" => status = value.clone(),
                "受众" => cate = value.clone(),
                "题材" => theme = value.clone(),
                "榜单" => rank_mode = value.clone(),
                _ => {}
            }
        }
    }
    
    // If rank mode is selected, use rank API (ignores other filters)
    let url = match rank_mode.as_str() {
        "1" => format!("{}/comic/rank/list?rank_type=0&by_time=0&page={}&size=20", V4_API_URL, page), // 日榜
        "2" => format!("{}/comic/rank/list?rank_type=0&by_time=1&page={}&size=20", V4_API_URL, page), // 周榜
        "3" => format!("{}/comic/rank/list?rank_type=0&by_time=2&page={}&size=20", V4_API_URL, page), // 月榜
        "4" => format!("{}/comic/rank/list?rank_type=0&by_time=3&page={}&size=20", V4_API_URL, page), // 总榜
        _ => format!(
            "{}/comic/filter/list?sortType={}&cate={}&status={}&zone={}&theme={}&page={}&size=20",
            V4_API_URL, sort_type, cate, status, zone, theme, page
        )
    };

    let mut response = get_api_request(&url)?.send()?;
    let json_val: serde_json::Value = response.get_json()?;

    // Parse based on API type
    if rank_mode != "0" {
        // Rank API returns array directly in data
        let data = json_val.get("data").ok_or_else(|| error!("Missing data"))?;
        json::parse_rank_list(data)
    } else {
        // Filter API returns object with data.comicList[]
        let data = json_val
            .get("data")
            .and_then(|d| d.get("comicList"))
            .ok_or_else(|| error!("Missing data.comicList"))?;
        json::parse_manga_list(data)
    }
}

/// Search manga by author name (complex hybrid search)
fn search_by_author(author: &str, page: i32) -> Result<MangaPageResult> {
    let encoded = encode_uri_component(author);
    
    // Helper: Check if author matches (handles "XX/YY" format)
    let author_matches = |manga_authors: &str| -> bool {
        if manga_authors.contains(author) {
            return true;
        }
        for part in manga_authors.split('/') {
            if part.trim().contains(author) || author.contains(part.trim()) {
                return true;
            }
        }
        false
    };
    
    let mut all_tag_ids: Vec<i64> = Vec::new();
    let mut keyword_manga: Vec<serde_json::Value> = Vec::new();
    let mut seen_authors: Vec<String> = Vec::new();
    
    // Step 1: Search for manga by author name
    let search_url = format!("{}/search/index?keyword={}&source=0&page=1&size=50", V4_API_URL, encoded);
    
    // Use authenticated request to access restricted content
    if let Ok(mut resp) = get_api_request(&search_url)?.send()
        && let Ok(json) = resp.get_json::<serde_json::Value>()
            && let Some(list) = json.get("data").and_then(|d| d.get("list")).and_then(|l| l.as_array()) {
                for manga in list {
                    let manga_authors = manga.get("authors").and_then(|a| a.as_str()).unwrap_or("");
                    
                    if author_matches(manga_authors) {
                        keyword_manga.push(manga.clone());
                        
                        let author_key = manga_authors.to_string();
                        if !seen_authors.contains(&author_key) {
                            seen_authors.push(author_key);
                            
                            if let Some(mid) = manga.get("id").and_then(|id| id.as_i64()) {
                                collect_author_tags(mid, &mut all_tag_ids)?;
                            }
                        }
                    }
                }
            }
    
    // Step 2: Fallback core name search if no results
    if all_tag_ids.is_empty() && keyword_manga.is_empty() {
        let core_name = author.trim_start_matches('◎').trim_start_matches('@').trim_start_matches('◯');
        let short_core = if core_name.chars().count() >= 4 {
            core_name.chars().take(2).collect::<String>()
        } else {
            core_name.to_string()
        };
        
        for core in [core_name, short_core.as_str()] {
            if core.is_empty() || core == author || !all_tag_ids.is_empty() { continue; }
            
            let core_encoded = encode_uri_component(core);
            let core_url = format!("{}/search/index?keyword={}&source=0&page=1&size=30", V4_API_URL, core_encoded);
            
            if let Ok(mut cresp) = get_api_request(&core_url)?.send()
                && let Ok(cjson) = cresp.get_json::<serde_json::Value>()
                    && let Some(clist) = cjson.get("data").and_then(|d| d.get("list")).and_then(|l| l.as_array()) {
                        for manga in clist {
                            if !all_tag_ids.is_empty() { break; }
                            
                            let manga_authors = manga.get("authors").and_then(|a| a.as_str()).unwrap_or("");
                            if manga_authors.contains(core) {
                                keyword_manga.push(manga.clone());
                                
                                let author_key = manga_authors.to_string();
                                if !seen_authors.contains(&author_key) {
                                    seen_authors.push(author_key);
                                    if let Some(mid) = manga.get("id").and_then(|id| id.as_i64()) {
                                        collect_author_tags(mid, &mut all_tag_ids)?;
                                    }
                                }
                            }
                        }
                    }
        }
    }
    
    // Step 3: Use tag_ids to get complete works (parallel requests)
    let mut tag_manga: Vec<serde_json::Value> = Vec::new();
    let mut tag_total = 0i32;
    
    if !all_tag_ids.is_empty() {
        // Build all requests
        let tag_requests: Vec<_> = all_tag_ids.iter()
            .filter_map(|tid| {
                let furl = format!("{}/comic/filter/list?theme={}&page={}&size=100", V4_API_URL, tid, page);
                net::get_request(&furl).ok()
            })
            .collect();
        
        // Send all requests in parallel
        let tag_responses = Request::send_all(tag_requests);
        
        // Process responses
        for resp_result in tag_responses {
            if let Ok(mut fr) = resp_result
                && let Ok(fj) = fr.get_json::<serde_json::Value>() {
                    if let Some(t) = fj.get("data").and_then(|d| d.get("total")).and_then(|v| v.as_i64()) {
                        tag_total = tag_total.max(t as i32);
                    }
                    if let Some(cl) = fj.get("data").and_then(|d| d.get("comicList")).and_then(|l| l.as_array()) {
                        tag_manga.extend(cl.iter().cloned());
                    }
                }
        }
    }
    
    // Step 4: Merge and deduplicate
    let mut seen_ids: Vec<i64> = Vec::new();
    let mut final_manga: Vec<serde_json::Value> = Vec::new();
    
    for m in &tag_manga {
        let id = m.get("comic_id").or_else(|| m.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
        if id > 0 && !seen_ids.contains(&id) {
            seen_ids.push(id);
            final_manga.push(m.clone());
        }
    }
    
    for m in &keyword_manga {
        let id = m.get("id").or_else(|| m.get("comic_id")).and_then(|v| v.as_i64()).unwrap_or(0);
        if id > 0 && !seen_ids.contains(&id) {
            seen_ids.push(id);
            final_manga.push(m.clone());
        }
    }
    
    if !final_manga.is_empty() {
        let fv = serde_json::Value::Array(final_manga.clone());
        let mut res = json::parse_manga_list(&fv)?;
        res.has_next_page = if tag_total > 0 { (page * 100) < tag_total } else { final_manga.len() >= 100 };
        return Ok(res);
    }
    
    Ok(MangaPageResult { entries: Vec::new(), has_next_page: false })
}

/// Helper to collect author tag_ids from manga detail
fn collect_author_tags(manga_id: i64, tag_ids: &mut Vec<i64>) -> Result<()> {
    let detail_url = format!("{}/comic/detail/{}?channel=android", V4_API_URL, manga_id);
    // Use authenticated request to access restricted content
    if let Ok(mut dr) = get_api_request(&detail_url)?.send()
        && let Ok(dj) = dr.get_json::<serde_json::Value>()
        && let Some(arr) = dj.get("data")
            .and_then(|d| d.get("data"))
            .and_then(|d| d.get("authors"))
            .and_then(|a| a.as_array())
    {
        for au in arr {
            if let Some(tid) = au.get("tag_id").and_then(|v| v.as_i64())
                && tid > 0 && !tag_ids.contains(&tid)
            {
                tag_ids.push(tid);
            }
        }
    }
    Ok(())
}

struct Zaimanhua;

impl Source for Zaimanhua {
    fn new() -> Self {
        // Try auto check-in on source init if logged in and not checked in today
        if let Some(token) = settings::get_token()
            && settings::get_auto_checkin()
            && !settings::has_checkin_flag()
            && let Ok(true) = net::check_in(&token)
        {
            settings::set_last_checkin("done");
        }
        Self
    }

    fn get_search_manga_list(
        &self,
        query: Option<String>,
        page: i32,
        filters: Vec<FilterValue>,
    ) -> Result<MangaPageResult> {
        // Check for author text filter first
        for filter in &filters {
            if let FilterValue::Text { id, value } = filter {
                if id == "author" {
                    return search_by_author(value, page);
                }
                // Non-author text filter: use keyword search
                return search_by_keyword(value, page);
            }
        }
        
        // Check for keyword search
        if let Some(ref keyword) = query
            && !keyword.is_empty()
        {
            return search_by_keyword(keyword, page);
        }
        
        // Default: browse with filters
        browse_with_filters(&filters, page)
    }

    fn get_manga_update(
        &self,
        mut manga: Manga,
        needs_details: bool,
        needs_chapters: bool,
    ) -> Result<Manga> {
        let url = format!(
            "{}/comic/detail/{}?channel=android",
            V4_API_URL, manga.key
        );

        // Use authenticated request to access comic_id=0 manga
        let mut response = get_api_request(&url)?.send()?;
        let json_val: serde_json::Value = response.get_json()?;

        // Check for API errors (e.g. deleted manga)
        if let Some(errno) = json_val.get("errno").and_then(|v| v.as_i64())
            && errno != 0
        {
            let errmsg = json_val.get("errmsg")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Err(error!("{}", errmsg));
        }

        let manga_data = json_val
            .get("data")
            .and_then(|d: &serde_json::Value| d.get("data"))
            .ok_or_else(|| error!("Missing data.data"))?;

        if needs_details {
            let details = json::parse_manga_details(manga_data, manga.key.clone())?;
            manga.title = details.title;
            manga.cover = details.cover;
            manga.description = details.description;
            manga.authors = details.authors;
            manga.tags = details.tags;
            manga.status = details.status;
            manga.content_rating = details.content_rating;
        }

        if needs_chapters {
            manga.chapters = Some(json::parse_chapters(manga_data, &manga.key)?);
        }

        Ok(manga)
    }

    fn get_page_list(&self, manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
        let parts: Vec<&str> = chapter.key.split('/').collect();
        let (comic_id, chapter_id) = if parts.len() == 2 {
            (parts[0], parts[1])
        } else {
            (manga.key.as_str(), chapter.key.as_str())
        };
        
        let url = format!(
            "{}/comic/chapter/{}/{}",
            V4_API_URL, comic_id, chapter_id
        );

        // Use authenticated request for chapter access
        let mut response = get_api_request(&url)?.send()?;
        let json_val: serde_json::Value = response.get_json()?;

        let inner_data = json_val
            .get("data")
            .and_then(|d| d.get("data"))
            .ok_or_else(|| error!("Missing data.data"))?;
        
        let page_urls = inner_data.get("page_url_hd")
            .or_else(|| inner_data.get("page_url"))
            .and_then(|p| p.as_array())
            .ok_or_else(|| error!("Missing page_url"))?;
        
        let mut pages: Vec<Page> = Vec::new();
        for url in page_urls.iter() {
            if let Some(url_str) = url.as_str() {
                pages.push(Page {
                    content: PageContent::url(url_str),
                    ..Default::default()
                });
            }
        }
        
        Ok(pages)
    }
}

impl ImageRequestProvider for Zaimanhua {
    fn get_image_request(
        &self,
        url: String,
        _context: Option<PageContext>,
    ) -> Result<Request> {
        Ok(Request::get(url)?
            .header("User-Agent", net::USER_AGENT)
            .header("Referer", BASE_URL))
    }
}

impl DeepLinkHandler for Zaimanhua {
    fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
        if url.contains("/manga/") || url.contains("/comic/") || url.contains("id=") {
            let id = if let Some(pos) = url.find("id=") {
                url[pos + 3..].split('&').next().unwrap_or("")
            } else {
                url.split('/').next_back().unwrap_or("")
            };
            
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                return Ok(Some(DeepLinkResult::Manga { key: id.into() }));
            }
        }
        Ok(None)
    }
}

impl BasicLoginHandler for Zaimanhua {
    fn handle_basic_login(&self, key: String, username: String, password: String) -> Result<bool> {
        if key != "login" {
            bail!("Invalid login key: `{key}`");
        }

        // Handle logout (empty username means logout)
        if username.is_empty() {
            settings::clear_all();
            return Ok(true);
        }

        if password.is_empty() {
            return Ok(false);
        }

        // Clear old account data before logging in with new credentials
        settings::clear_all();
        
        settings::set_username(&username);
        settings::set_password(&password);

        match net::login(&username, &password) {
            Ok(Some(token)) => {
                settings::set_token(&token);
                // Auto check-in after login (if enabled and not already done)
                if settings::get_auto_checkin()
                    && !settings::has_checkin_flag()
                    && let Ok(true) = net::check_in(&token)
                {
                    settings::set_last_checkin("done");
                }
                Ok(true)
            }
            _ => Ok(false)
        }
    }
}

impl NotificationHandler for Zaimanhua {
    fn handle_notification(&self, notification: String) {
        if notification == "checkin"
            && let Some(token) = settings::get_token()
        {
            let _ = net::check_in(&token);
        }
    }
}

// === Dynamic Settings for User Info Display ===

impl DynamicSettings for Zaimanhua {
    fn get_dynamic_settings(&self) -> Result<Vec<Setting>> {
        let mut settings_list: Vec<Setting> = Vec::new();
        
        // Check login status
        let is_logged_in = settings::get_token().is_some();
        
        // Try to get user info if logged in
        let user_info_opt = if is_logged_in {
            if let Some(token) = settings::get_token() {
                if let Ok(user_data) = net::get_user_info(&token) {
                    user_data.get("data")
                        .and_then(|d| d.get("userInfo"))
                        .cloned()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        
        // Prepare checkin subtitle
        let checkin_subtitle = user_info_opt.as_ref()
            .map(|info| {
                let is_signed = info.get("is_sign").and_then(|v| v.as_bool()).unwrap_or(false);
                if is_signed { "今日已签到" } else { "今日未签到" }
            });
        
        // Account Group
        let mut account_items: Vec<Setting> = Vec::new();
        
        // Login (with logout notification)
        account_items.push(
            LoginSetting {
                key: "login".into(),
                title: "登录".into(),
                notification: Some("login".into()),  // This fires on both login and logout
                method: LoginMethod::Basic,
                refreshes: Some(vec!["settings".into(), "content".into(), "listings".into()]),
                ..Default::default()
            }.into()
        );
        
        // Auto check-in (always show, but with status subtitle only when logged in)
        account_items.push(
            ToggleSetting {
                key: "autoCheckin".into(),
                title: "自动签到".into(),
                subtitle: checkin_subtitle.map(|s| s.into()),
                default: false,
                ..Default::default()
            }.into()
        );
        
        // Enhanced mode (only show when logged in)
        if is_logged_in {
            account_items.push(
                ToggleSetting {
                    key: "enhancedMode".into(),
                    title: "增强浏览".into(),
                    subtitle: Some("获取更多内容".into()),
                    default: false,
                    refreshes: Some(vec!["content".into(), "listings".into()]),  // Refresh content when toggled
                    ..Default::default()
                }.into()
            );
        }
        
        settings_list.push(
            GroupSetting {
                key: "account".into(),
                title: "账号".into(),
                items: account_items,
                ..Default::default()
            }.into()
        );
        
        // User info group (only if we successfully got user info)
        if let Some(user_info) = user_info_opt {
            // Extract info
            let username = user_info.get("username")
                .or_else(|| user_info.get("nickname"))
                .and_then(|v| v.as_str())
                .unwrap_or("未知用户");
            
            let level = user_info.get("user_level")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            
            // Build info footer
            let footer_text = format!(
                "用户：{} | 等级：Lv.{}",
                username, level
            );
            
            // Add user info group
            settings_list.push(
                GroupSetting {
                    key: "userInfo".into(),
                    title: "账号信息".into(),
                    items: Vec::new(),
                    footer: Some(footer_text.into()),
                    ..Default::default()
                }.into()
            );
        }
        
        Ok(settings_list)
    }
}

impl Home for Zaimanhua {
    fn get_home(&self) -> Result<HomeLayout> {
        home::get_home_layout()
    }
}

impl ListingProvider for Zaimanhua {
    fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
        // Handle rank listings (use rank API)
        if listing.id == "rank-monthly" {
            let url = format!(
                "{}/comic/rank/list?rank_type=0&by_time=2&page={}&size=20",
                V4_API_URL, page
            );
            let mut response = get_api_request(&url)?.send()?;
            let data: serde_json::Value = response.get_json()?;
            let list = data.get("data")
                .ok_or_else(|| aidoku::error!("No data in rank response"))?;
            return json::parse_rank_list(list);
        }
        
        // Handle filter-based listings
        let url = match listing.id.as_str() {
            "latest" => format!(
                "{}/comic/filter/list?sortType=1&page={}&size=20",
                V4_API_URL, page
            ),
            "ongoing" => format!(
                "{}/comic/filter/list?status=2309&page={}&size=20",
                V4_API_URL, page
            ),
            "completed" => format!(
                "{}/comic/filter/list?status=2310&page={}&size=20",
                V4_API_URL, page
            ),
            "short" => format!(
                "{}/comic/filter/list?status=29205&page={}&size=20",
                V4_API_URL, page
            ),
            // Audience categories (dynamic listings) - no sortType to match Home page
            "shounen" => format!(
                "{}/comic/filter/list?cate=3262&page={}&size=20",
                V4_API_URL, page
            ),
            "shoujo" => format!(
                "{}/comic/filter/list?cate=3263&page={}&size=20",
                V4_API_URL, page
            ),
            "seinen" => format!(
                "{}/comic/filter/list?cate=3264&page={}&size=20",
                V4_API_URL, page
            ),
            "josei" => format!(
                "{}/comic/filter/list?cate=13626&page={}&size=20",
                V4_API_URL, page
            ),
            // 订阅列表 - 需要登录和增强模式
            "subscribe" => {
                let token = settings::get_token()
                    .ok_or_else(|| aidoku::error!("请先登录以查看订阅列表"))?;
                
                if !settings::get_enhanced_mode() {
                    return Err(aidoku::error!("请开启增强模式以使用订阅功能"));
                }
                
                let url = format!(
                    "{}/comic/sub/list?status=0&firstLetter=&page={}&size=50",
                    V4_API_URL, page
                );
                
                let mut response = net::auth_request(&url, &token)?.send()?;
                let json_val: serde_json::Value = response.get_json()?;
                
                let data = json_val.get("data")
                    .ok_or_else(|| aidoku::error!("Invalid subscribe response"))?;
                
                return json::parse_subscribe_list(data);
            },
            _ => return Err(aidoku::error!("Unknown listing: {}", listing.id)),
        };

        let request = get_api_request(&url)?;
        let mut response = request.send()?;
        let data: serde_json::Value = response.get_json()?;

        let list = data.get("data")
            .and_then(|d| d.get("comicList"))
            .ok_or_else(|| aidoku::error!("No comicList in filter response"))?;
        
        json::parse_manga_list(list)
    }
}

register_source!(
    Zaimanhua,
    Home,
    ListingProvider,
    ImageRequestProvider,
    DeepLinkHandler,
    BasicLoginHandler,
    NotificationHandler,
    DynamicSettings
);

