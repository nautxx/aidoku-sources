use crate::{BASE_URL, REFERER, TRUST_COOKIE_KEY, USER_AGENT, models::parse_chapter};
use aidoku::{
	Manga, MangaPageResult, MangaWithChapter, Result,
	alloc::{String, Vec},
	imports::{
		defaults::{DefaultValue, defaults_get, defaults_set},
		html::Document,
		js::{Cookie, WebView, WebViewUserScript},
		net::Request,
		std::sleep,
	},
	prelude::*,
};
use serde::de::DeserializeOwned;

const COOKIE_KEY: &str = "cookie";
const USER_AGENT_KEY: &str = "userAgent";
const GUARD_TIMEOUT_SECS: i32 = 20;
// the guard pauses while the page is hidden and runs on animation frames,
// but a web view that isn't on screen is always hidden and never gets frames
const VISIBLE_PAGE_JS: &str = "\
Object.defineProperty(document, 'hidden', { get: () => false });\
Object.defineProperty(document, 'visibilityState', { get: () => 'visible' });\
window.requestAnimationFrame = (callback) => setTimeout(() => callback(performance.now()), 16);";

pub fn get_html(url: &str) -> Result<Document> {
	fetch_html(|| Ok(Request::get(url)?))
}

pub fn post_html(url: &str, body: &str) -> Result<Document> {
	fetch_html(|| {
		Ok(Request::post(url)?
			.header("Content-Type", "application/x-www-form-urlencoded")
			.body(body))
	})
}

fn fetch_html(request: impl Fn() -> Result<Request>) -> Result<Document> {
	match send(request()?) {
		Ok(Some(html)) => return Ok(html),
		// with no saved cookies, a failed request isn't something the guard caused
		Err(error) if defaults_get::<String>(COOKIE_KEY).is_none() => return Err(error),
		// blocked, or failed while sending saved cookies that may have gone stale
		_ => {}
	}
	// the site's guard sets its trust cookie with javascript, so pass it in a web view and retry
	solve_guard()?;
	send(request()?)?.ok_or(error!("Failed to pass BatCave's browser check"))
}

/// Returns `None` if the request was stopped by the guard or cloudflare.
fn send(request: Request) -> Result<Option<Document>> {
	let user_agent = defaults_get::<String>(USER_AGENT_KEY).unwrap_or_else(|| USER_AGENT.into());
	let mut request = request
		.header("Referer", REFERER)
		.header("User-Agent", &user_agent);
	if let Some(cookie) = defaults_get::<String>(COOKIE_KEY) {
		request = request.header("Cookie", &cookie);
	}
	let response = request.send()?;
	let html = response.get_html()?;

	let blocked = response.get_url().is_some_and(|url| url.contains("/_c?"))
		|| response
			.get_header("cf-mitigated")
			.is_some_and(|value| value == "challenge")
		|| html
			.select_first("title")
			.and_then(|el| el.text())
			.is_none_or(|s| s.is_empty());
	Ok((!blocked).then_some(html))
}

fn solve_guard() -> Result<()> {
	// forget the old cookies so that later requests don't keep sending them if this fails
	defaults_set(COOKIE_KEY, DefaultValue::Null);
	defaults_set(USER_AGENT_KEY, DefaultValue::Null);

	let web_view = WebView::new();
	// remove the old trust cookie so that only a fresh one counts as passing
	for cookie in web_view.get_cookies()? {
		if cookie.name == TRUST_COOKIE_KEY {
			web_view.delete_cookie(cookie)?;
		}
	}
	web_view.add_user_script(WebViewUserScript::new(VISIBLE_PAGE_JS.into()))?;
	web_view.load(Request::get(BASE_URL)?)?;

	for _ in 0..GUARD_TIMEOUT_SECS {
		sleep(1);
		let cookies: Vec<Cookie> = web_view
			.get_cookies()?
			.into_iter()
			.filter(|cookie| {
				let domain = cookie.domain.trim_start_matches('.');
				domain == "batcave.biz" || domain.ends_with(".batcave.biz")
			})
			.collect();
		if cookies.iter().any(|cookie| cookie.name == TRUST_COOKIE_KEY) {
			let cookie = cookies
				.iter()
				.map(|cookie| format!("{}={}", cookie.name, cookie.value))
				.collect::<Vec<_>>()
				.join("; ");
			// send the same user agent as the web view that passed the check
			let user_agent = web_view.eval("navigator.userAgent")?;
			defaults_set(COOKIE_KEY, DefaultValue::String(cookie));
			defaults_set(USER_AGENT_KEY, DefaultValue::String(user_agent));
			return Ok(());
		}
	}

	bail!("Timed out waiting for BatCave's browser check")
}

pub fn comix_url(page: i32) -> String {
	if page > 1 {
		format!("{BASE_URL}/comix/page/{page}/")
	} else {
		format!("{BASE_URL}/comix/")
	}
}

/// Builds the form body that sorts a list, where `list` is "cat_1" for the comic list
/// and "xfilter" for filtered lists.
pub fn sort_body(sort_by: &str, ascending: bool, list: &str) -> String {
	let direction = if ascending { "asc" } else { "desc" };
	format!(
		"dlenewssortby={sort_by}&dledirection={direction}\
		&set_new_sort=dle_sort_{list}&set_direction_sort=dle_direction_{list}"
	)
}

/// Parses the object an inline script assigns, like `window.__DATA__ = {...};`.
/// The trailing semicolon is optional, since the filter data script leaves it out.
pub fn parse_script_json<T: DeserializeOwned>(html: &Document, variable: &str) -> Option<T> {
	html.select("script")?.find_map(|script| {
		let data = script.data()?;
		let json = data
			.trim()
			.strip_prefix(variable)?
			.trim_start()
			.strip_prefix('=')?
			.trim()
			.trim_end_matches(';');
		serde_json::from_str(json).ok()
	})
}

pub fn latest_url(page: i32) -> String {
	if page > 1 {
		format!("{BASE_URL}/page/{page}/")
	} else {
		format!("{BASE_URL}/")
	}
}

/// Parses a page of the comic list, which search and filtering also use.
pub fn parse_manga_list(html: &Document) -> MangaPageResult {
	let entries = html
		.select("#dle-content > .readed")
		.map(|elements| {
			elements
				.filter_map(|element| {
					let link = element.select_first(".readed__title > a")?;
					let url = link.attr("abs:href")?;
					Some(Manga {
						key: url.strip_prefix(BASE_URL)?.into(),
						cover: element.select_first("img")?.attr("abs:data-src"),
						title: link.own_text()?,
						url: Some(url),
						..Default::default()
					})
				})
				.collect()
		})
		.unwrap_or_default();

	let has_next_page = html
		.select_first("div.pagination__pages")
		.and_then(|el| el.children().next_back())
		.map(|child| child.tag_name().as_deref() == Some("a"))
		.unwrap_or_default();

	MangaPageResult {
		entries,
		has_next_page,
	}
}

/// Parses the newest releases on the home page, with each comic's newest chapter.
pub fn parse_latest(html: &Document) -> (Vec<MangaWithChapter>, bool) {
	let entries = html
		.select("#content-load > .latest.grid-item")
		.map(|elements| {
			elements
				.filter_map(|element| {
					let link = element.select_first(".latest__title > a")?;
					let url = link.attr("abs:href")?;
					let title = link.text()?;

					// links look like "/reader/33516/275180" with the text "15.09.2026 - Title #17"
					let chapter_link = element.select_first(".latest__chapter > a")?;
					let chapter_url = chapter_link.attr("abs:href")?;
					let (news_id, id) = chapter_url
						.strip_prefix(BASE_URL)?
						.strip_prefix("/reader/")?
						.split_once('/')?;
					let chapter_text = chapter_link.text()?;
					let (date, chapter_title) = chapter_text.split_once(" - ")?;
					let chapter = parse_chapter(
						news_id.parse().ok()?,
						id.parse().ok()?,
						date,
						chapter_title,
						&title,
					);

					Some(MangaWithChapter {
						manga: Manga {
							key: url.strip_prefix(BASE_URL)?.into(),
							cover: element
								.select_first(".latest__img img")
								.and_then(|img| img.attr("abs:src")),
							title,
							url: Some(url),
							..Default::default()
						},
						chapter,
					})
				})
				.collect()
		})
		.unwrap_or_default();

	let has_next_page = html.select_first("li.pagination a[href]").is_some();
	(entries, has_next_page)
}
