use crate::models;
use crate::net;
use crate::settings;
use aidoku::{
	Manga, MangaPageResult, Result,
	alloc::{String, Vec, format, string::ToString, vec},
	imports::net::Request,
};
use hashbrown::HashSet;

// === URL/ID Parsing Helpers ===

/// Check if an item should be hidden based on settings and its hidden flag
pub fn should_hide_item(hidden_status: Option<i32>) -> bool {
	// If user wants to see hidden content (Enhanced Mode + Deep Search Toggle), never hide.
	if settings::show_hidden_content() {
		return false;
	}
	// Otherwise, hide if the item is explicitly marked as hidden (1).
	hidden_status.unwrap_or(0) == 1
}

/// Parse manga ID from keyword (numeric ID or URL)
fn parse_manga_id(keyword: &str) -> Option<String> {
	let trimmed = keyword.trim();
	
	// Direct numeric ID
	if trimmed.chars().all(|c| c.is_ascii_digit()) && !trimmed.is_empty() {
		return Some(trimmed.to_string());
	}
	
	// URL patterns: /details/ID or /comic/ID
	if trimmed.contains("zaimanhua.com") {
		// Extract ID from URL like https://manhua.zaimanhua.com/details/70258
		if let Some(pos) = trimmed.rfind('/') {
			let after = &trimmed[pos + 1..];
			let id_part: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
			if !id_part.is_empty() {
				return Some(id_part);
			}
		}
	}
	
	None
}

/// Fetch manga directly by ID (bypasses search API)
fn fetch_manga_by_id(id: &str) -> Option<Manga> {
	let url = net::urls::detail(id.parse::<i64>().unwrap_or(0));
	let token = settings::get_current_token();
	
	let response: models::ApiResponse<models::DetailData> = 
		net::auth_request(&url, token.as_deref()).ok()?.json_owned().ok()?;
	
	if response.errno.unwrap_or(0) != 0 {
		return None;
	}
	
	let detail = response.data?.data?;
	Some(detail.into_manga(id.to_string()))
}

// === Search Logic ===

pub fn search_by_keyword(keyword: &str, page: i32) -> Result<MangaPageResult> {
	if keyword.trim().is_empty() {
		return Ok(MangaPageResult::default());
	}

	// === URL/ID Parsing (Direct Access) ===
	// If the keyword is a numeric ID or a recognized URL, try to fetch it directly.
	// This bypasses the Search API which might hide some content.
	if page == 1
		&& let Some(id) = parse_manga_id(keyword)
		&& let Some(manga) = fetch_manga_by_id(&id)
	{
		return Ok(MangaPageResult {
			entries: vec![manga],
			has_next_page: false,
		});
	}

	let keyword_lower = keyword.to_lowercase();
	let mut token = settings::get_current_token();
	let mut search_data: Option<models::SearchData> = None;
	let mut hidden_items: Vec<models::FilterItem> = Vec::new();
	let mut hidden_has_next = false;

	// === Parallel Search Execution ===
	// Batch the main search request together with hidden content scans (if enabled).
	// This parallelism significantly reduces total latency.
	for retry_count in 0..2 {
		let token_ref = token.as_deref();
		let mut requests = Vec::new();

		let search_url = net::urls::search(keyword, page);
		requests.push(
			net::auth_request(&search_url, token_ref)
				.unwrap_or_else(|_| net::get_request(&search_url).expect("Invalid URL"))
		);

		// Hidden Content scans (paginated)
		let hidden_start_page = (page - 1) * 5 + 1;
		let should_scan_hidden = settings::show_hidden_content();

		if should_scan_hidden {
			for i in 0..5 {
				let p = hidden_start_page + i;
				let url = net::urls::filter_latest(p);
				requests.push(
					net::auth_request(&url, token_ref)
						.unwrap_or_else(|_| net::get_request(&url).expect("Invalid URL"))
				);
			}
		}

		let responses = Request::send_all(requests);
		if responses.is_empty() { break; }

		// Parse responses & check for Auth Error (errno 99)
		let mut needs_retry = false;
		let mut parsing_search_data = None;
		let mut parsing_hidden_items = Vec::new();

		let mut response_iter = responses.into_iter();

		// Handle Search Response (Index 0)
		if let Some(Ok(resp)) = response_iter.next()
			&& let Ok(api_resp) = resp.get_json_owned::<models::ApiResponse<models::SearchData>>()
		{
			if api_resp.errno.unwrap_or(0) == 99 {
				needs_retry = true;
			} else {
				parsing_search_data = api_resp.data;
			}
		}

		// Handle Hidden Responses (Index 1..)
		if should_scan_hidden && !needs_retry {
			for resp in response_iter.flatten() {
				if let Ok(api_resp) = resp.get_json_owned::<models::ApiResponse<models::FilterData>>() {
					if api_resp.errno.unwrap_or(0) == 99 {
						needs_retry = true;
						break;
					}
					if let Some(valid_data) = api_resp.data {
						parsing_hidden_items.extend(valid_data.comic_list);
					}
				}
			}
		}

		if needs_retry {
			if retry_count == 0
				&& let Ok(Some(new_token)) = net::try_refresh_token()
			{
				token = Some(new_token);
				continue; // Retry loop with new token
			}
			// If login fails or already retried, stop/fail gracefully
			break;
		}

		// Success - Commit data
		search_data = parsing_search_data;
		hidden_items = parsing_hidden_items;
		break;
	}

	// === Result Merging ===

	let mut results: Vec<Manga> = if let Some(data) = search_data {
		data.list.into_iter().map(Into::into).collect()
	} else {
		Vec::new()
	};

	let existing_ids: HashSet<String> = results.iter().map(|m| m.key.clone()).collect();
	let mut hidden_count = 0;

	if !hidden_items.is_empty() {
		// Filter hidden items locally
		for item in hidden_items {
			let sid = item.id.to_string();
			if existing_ids.contains(&sid) { continue; }

			let name_lower = item.name.to_lowercase();
			let auth_lower = item.authors.as_deref().unwrap_or("").to_lowercase();

			if name_lower.contains(&keyword_lower) || auth_lower.contains(&keyword_lower) {
				results.push(item.into());
				hidden_count += 1;
			}
		}
		
		if hidden_count >= 1 {
			hidden_has_next = true;
		}
	}

	let has_next_page = !results.is_empty() || hidden_has_next;

	Ok(MangaPageResult {
		entries: results,
		has_next_page,
	})
}

pub fn search_by_author(author: &str, page: i32) -> Result<MangaPageResult> {
	if author.trim().is_empty() {
		return Ok(MangaPageResult::default());
	}
	let mut strict_ids: Vec<i64> = Vec::new();
	let mut partial_ids: Vec<i64> = Vec::new();
	let mut seen_authors: Vec<String> = Vec::new();
	let mut strict_manga: Vec<Manga> = Vec::new();

	let token = settings::get_current_token();
	let token_ref = token.as_deref();

	// === Direct Search ===
	let mut keyword_manga =
		collect_tags_from_search(author, author, &mut seen_authors, &mut strict_ids, &mut partial_ids, &mut strict_manga, token_ref)?;

	// === Global Strict Priority Decision ===
	let exact_match_found = !strict_ids.is_empty();

	let direct_ids = if exact_match_found {
		strict_ids
	} else {
		partial_ids // Fallback to partials if NO strict found
	};

	// === Fuzzy Fallback ===
	// Only engaged if NO tags found at all.
	let final_tag_ids = if direct_ids.is_empty() && keyword_manga.is_empty() {
		let mut fuzzy_strict = Vec::new();
		let mut fuzzy_partial = Vec::new();
		let fuzzy_manga = try_fuzzy_author_search(author, &mut seen_authors, &mut fuzzy_strict, &mut fuzzy_partial, &mut strict_manga, token_ref)?;
		
		keyword_manga.extend(fuzzy_manga);
		
		if !fuzzy_strict.is_empty() {
			fuzzy_strict
		} else {
			fuzzy_partial
		}
	} else {
		direct_ids
	};

	// === Tag Search & Merge ===
	let (tag_manga, tag_total) = fetch_manga_by_tags(&final_tag_ids, page, token_ref)?;

	let mut merger = MangaMerger::new();

	if settings::show_hidden_content() {
		let hidden_manga = scan_hidden_content(page, author, merger.seen_ids_mut(), token_ref, exact_match_found);
		merger.extend(hidden_manga);
	}

	// Add results based on Strict Priority
	let tag_iter = tag_manga.into_iter().map(|i| -> Manga { i.into() });

	// === Result Merging ===
	// If strict matches exist, enable "Strict Mode":
	// 1. Only include results that were strictly matched (ID-based or Name-based).
	// 2. Discard fuzzy candidates from the initial keyword search to eliminate noise.

	if exact_match_found {
		// Strict Mode: Use Token/Tag results AND Verified Strict Candidates.
		merger.extend(tag_iter.chain(strict_manga));
	} else {
		// Fallback/Fuzzy Mode: Merge everything.
		let keyword_iter = keyword_manga.into_iter().map(|i| -> Manga { i.into() });
		merger.extend(tag_iter.chain(keyword_iter));
	}

	if !merger.is_empty() {
		let has_next = if tag_total > 0 {
			tag_total >= 100
		} else {
			merger.len() >= 100
		};
		return Ok(MangaPageResult {
			entries: merger.finish(),
			has_next_page: has_next,
		});
	}

	Ok(MangaPageResult::default())
}

// === Decomposed Helpers ===

fn collect_tags_from_search(
	keyword: &str,
	target_author: &str,
	seen_authors: &mut Vec<String>,
	strict_ids: &mut Vec<i64>,
	partial_ids: &mut Vec<i64>,
	strict_manga: &mut Vec<Manga>,
	token: Option<&str>,
) -> Result<Vec<models::SearchItem>> {
	let mut matched_items = Vec::new();

	let url = format!("{}&source=0", net::urls::search(keyword, 1));

	if let Ok(response) = net::send_authed_request::<models::SearchData>(&url, token)
		&& let Some(data) = response.data
	{
		// Identify Candidate Items locally
		let candidates: Vec<&models::SearchItem> = data.list
			.iter()
			.filter(|item| item.matches_author(target_author))
			.collect();

		if candidates.is_empty() {
			return Ok(matched_items);
		}

		// === Batch Verify Candidates ===
		// Map candidates to requests, execute in parallel, correlate results.
		let requests: Vec<Request> = candidates
			.iter()
			.map(|item| {
				let url = net::urls::detail(item.id);
				net::auth_request(&url, token)
					.unwrap_or_else(|_| net::get_request(&url).expect("Invalid URL"))
			})
			.collect();

		let responses = Request::send_all(requests);

		// Process Responses
		for (i, response_result) in responses.into_iter().enumerate() {
			let candidate = candidates[i];
			let manga_authors = candidate.authors.as_deref().unwrap_or("");
			let author_key = manga_authors.to_string();

			// Ensure "seen_authors" constraint logic is respected
			if seen_authors.contains(&author_key) {
				matched_items.push(candidate.clone());
				continue;
			}

			// Validate response
			if let Ok(resp) = response_result
				&& let Ok(api_resp) = resp.get_json_owned::<models::ApiResponse<models::DetailData>>()
				&& let Some(data_root) = api_resp.data
				&& let Some(detail) = data_root.data
			{
				seen_authors.push(author_key);

				// Use pure logic extractor (returns new struct)
				let result = extract_author_tags_from_detail(&detail, target_author);
				strict_ids.extend(result.strict_ids);
				partial_ids.extend(result.partial_ids);

				match result.match_type {
					MatchResult::Strict => {
						matched_items.push(candidate.clone());
						strict_manga.push(candidate.clone().into());
					},
					MatchResult::Partial => {
						matched_items.push(candidate.clone());
					},
					MatchResult::None => {}
				}
			}
		}
	}
	Ok(matched_items)
}

fn try_fuzzy_author_search(
	author: &str,
	seen_authors: &mut Vec<String>,
	strict_ids: &mut Vec<i64>,
	partial_ids: &mut Vec<i64>,
	strict_manga: &mut Vec<Manga>,
	token: Option<&str>,
) -> Result<Vec<models::SearchItem>> {
	let core_name = author;
	let short_core = if core_name.chars().count() >= 4 {
		core_name.chars().take(2).collect::<String>()
	} else {
		core_name.to_string()
	};

	let mut matched_items = Vec::new();

	for core in [core_name, short_core.as_str()] {
		if core.is_empty() || core == author || (!strict_ids.is_empty() || !partial_ids.is_empty()) {
			continue;
		}

		let keywords = collect_tags_from_search(core, author, seen_authors, strict_ids, partial_ids, strict_manga, token)?;
		matched_items.extend(keywords);

		if !strict_ids.is_empty() || !partial_ids.is_empty() {
			break;
		}
	}
	Ok(matched_items)
}

fn fetch_manga_by_tags(tag_ids: &[i64], page: i32, token: Option<&str>) -> Result<(Vec<models::FilterItem>, i32)> {
	if tag_ids.is_empty() {
		return Ok((Vec::new(), 0));
	}

	let tag_requests: Vec<_> = tag_ids
		.iter()
		.filter_map(|tid| {
			let url = net::urls::filter_theme(*tid, page);
			net::auth_request(&url, token).ok()
		})
		.collect();

	let items: Vec<models::FilterItem> = Request::send_all(tag_requests)
		.into_iter()
		.flatten()
		.filter_map(|resp| {
			resp.get_json_owned::<models::ApiResponse<models::FilterData>>()
				.ok()
				.and_then(|r| r.data)
		})
		.flat_map(|data| data.comic_list)
		.collect();

	let total = items.len() as i32;
	Ok((items, total))
}

fn scan_hidden_content(
	page: i32,
	target_author: &str,
	seen_ids: &mut HashSet<i64>,
	token: Option<&str>,
	strict_mode: bool,
) -> Vec<Manga> {
	let mut found_manga = Vec::new();
	let hidden_start_page = (page - 1) * 5 + 1;
	let scanner = net::HiddenContentScanner::new(hidden_start_page, 3, token);

	for items in scanner {
		let mut batch_found_any = false;
		for item in items {
			let is_match = if strict_mode {
				// Exact match logic
				if let Some(auth) = &item.authors {
					auth.split(',').any(|a| a.trim() == target_author)
				} else {
					false
				}
			} else {
				// Original fuzzy logic
				item.matches_author(target_author)
			};

			if is_match && !seen_ids.contains(&item.id) {
				seen_ids.insert(item.id);
				found_manga.push(item.into());
				batch_found_any = true;
			}
		}

		if batch_found_any {
			break;
		}
	}
	found_manga
}

// === Helper Functions ===

#[derive(PartialEq, Default)]
enum MatchResult {
	Strict,
	Partial,
	#[default]
	None,
}

#[derive(Default)]
struct AuthorMatchResult {
	strict_ids: Vec<i64>,
	partial_ids: Vec<i64>,
	match_type: MatchResult,
}

/// Encapsulates result merging with ID-based deduplication.
/// Uses i64 IDs for deduplication to avoid string allocation overhead.
pub struct MangaMerger {
	seen_ids: HashSet<i64>,
	results: Vec<Manga>,
}

impl MangaMerger {
	pub fn new() -> Self {
		Self {
			seen_ids: HashSet::new(),
			results: Vec::new(),
		}
	}

	/// Try to add a manga by its numeric ID. Returns true if added (not duplicate).
	pub fn try_add(&mut self, id: i64, manga: Manga) -> bool {
		if id > 0 && !self.seen_ids.contains(&id) {
			self.seen_ids.insert(id);
			self.results.push(manga);
			true
		} else {
			false
		}
	}

	/// Add a manga, parsing its key as i64. Returns true if added.
	pub fn add(&mut self, manga: Manga) -> bool {
		if let Ok(id) = manga.key.parse::<i64>() {
			self.try_add(id, manga)
		} else {
			false
		}
	}

	/// Extend with an iterator of Manga, deduplicating by parsed key.
	pub fn extend<I: IntoIterator<Item = Manga>>(&mut self, iter: I) {
		for manga in iter {
			self.add(manga);
		}
	}

	/// Consume and return the deduplicated results.
	pub fn finish(self) -> Vec<Manga> {
		self.results
	}

	/// Get current result count.
	pub fn len(&self) -> usize {
		self.results.len()
	}

	/// Check if results are empty.
	pub fn is_empty(&self) -> bool {
		self.results.is_empty()
	}

	/// Get mutable access to the internal seen_ids set (for compatibility with existing code).
	pub fn seen_ids_mut(&mut self) -> &mut HashSet<i64> {
		&mut self.seen_ids
	}
}

/// Pure function: extracts author tag IDs from manga detail.
fn extract_author_tags_from_detail(
	detail: &models::MangaDetail,
	target_author: &str,
) -> AuthorMatchResult {
	let Some(authors) = &detail.authors else {
		return AuthorMatchResult::default();
	};

	let mut seen_strict: HashSet<i64> = HashSet::new();
	let mut seen_partial: HashSet<i64> = HashSet::new();

	// Single-pass: collect strict and partial matches
	let (strict_ids, partial_ids): (Vec<i64>, Vec<i64>) = authors
		.iter()
		.filter_map(|author| {
			let name = author.tag_name.as_ref()?;
			let tid = author.tag_id.filter(|&id| id > 0)?;
			if name.trim().is_empty() {
				return None;
			}

			if name == target_author && seen_strict.insert(tid) {
				return Some((Some(tid), None)); // Strict match
			} else if (name.contains(target_author) || target_author.contains(name.as_str()))
				&& seen_partial.insert(tid)
			{
				return Some((None, Some(tid))); // Partial match
			}
			None
		})
		.fold((Vec::new(), Vec::new()), |(mut s, mut p), (strict, partial)| {
			if let Some(id) = strict { s.push(id); }
			if let Some(id) = partial { p.push(id); }
			(s, p)
		});

	let match_type = if !strict_ids.is_empty() {
		MatchResult::Strict
	} else if !partial_ids.is_empty() {
		MatchResult::Partial
	} else {
		MatchResult::None
	};

	AuthorMatchResult { strict_ids, partial_ids, match_type }
}
