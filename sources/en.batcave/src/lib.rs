#![no_std]
use aidoku::{
	Chapter, ContentRating, DeepLinkHandler, DeepLinkResult, DynamicFilters, Filter, FilterValue,
	ImageRequestProvider, Listing, ListingProvider, Manga, MangaPageResult, MangaStatus,
	MultiSelectFilter, Page, PageContent, Result, Source, Viewer,
	alloc::{String, Vec, string::ToString, vec},
	helpers::uri::encode_uri_component,
	imports::net::Request,
	prelude::*,
};

mod helpers;
mod home;
mod models;

use helpers::*;
use models::*;

const BASE_URL: &str = "https://batcave.biz";
const REFERER: &str = "https://batcave.biz/";
const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) \
                          AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 \
                          Mobile/15E148 Safari/604.1";
const TRUST_COOKIE_KEY: &str = "__guard_trust";

const LATEST_LISTING: &str = "latest";
const TOP_RATED_LISTING: &str = "top-rated";
const JUST_ADDED_LISTING: &str = "just-added";
// the same ids and names as the listings in res/source.json
const LISTING_NAMES: [(&str, &str); 3] = [
	(LATEST_LISTING, "Newest Releases"),
	(TOP_RATED_LISTING, "Top Rated"),
	(JUST_ADDED_LISTING, "Just Added"),
];

// the site's names for the sort options in res/filters.json, in the same order
const SORT_OPTIONS: [&str; 6] = [
	"date",
	"editdate",
	"rating",
	"news_read",
	"comm_num",
	"title",
];

struct BatCave;

impl Source for BatCave {
	fn new() -> Self {
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		if let Some(query) = query {
			let url = format!(
				"{BASE_URL}/search/{}/page/{page}/",
				encode_uri_component(query)
			);
			return Ok(parse_manga_list(&get_html(&url)?));
		}

		let mut filters_vec = Vec::<String>::new();
		let mut sort = None;
		for filter in filters {
			match filter {
				FilterValue::Range { from, to, .. } => {
					if let Some(from) = from {
						filters_vec.push(format!("y[from]={}", from));
					}
					if let Some(to) = to {
						filters_vec.push(format!("y[to]={}", to));
					}
				}
				FilterValue::MultiSelect { id, included, .. } if !included.is_empty() => {
					let key = if id == "publisher" { "p" } else { "g" };
					filters_vec.push(format!("{key}={}", included.join(",")));
				}
				FilterValue::Text { id, value } if !value.is_empty() => {
					let key = if id == "artist" { "a" } else { "w" };
					filters_vec.push(format!("{key}={}", encode_uri_component(value)));
				}
				FilterValue::Sort {
					index, ascending, ..
				} => sort = Some((index, ascending)),
				_ => {}
			}
		}

		// the site takes the sort under a different name for filtered lists
		let (url, sort_list) = if filters_vec.is_empty() {
			(comix_url(page), "cat_1")
		} else {
			let filters = filters_vec.join("/");
			(
				format!("{BASE_URL}/ComicList/{filters}/page/{page}/"),
				"xfilter",
			)
		};
		let html = match sort {
			// newest first is the site's default, so only other sorts are sent
			Some((index, ascending)) if index > 0 || ascending => {
				let sort_by = SORT_OPTIONS.get(index as usize).unwrap_or(&"date");
				post_html(&url, &sort_body(sort_by, ascending, sort_list))?
			}
			_ => get_html(&url)?,
		};
		Ok(parse_manga_list(&html))
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = format!("{BASE_URL}{}", manga.key);
		let html = get_html(&url)?;

		if needs_details {
			manga.title = html
				.select_first("header h1")
				.and_then(|x| x.text())
				.unwrap_or_default();

			manga.description = html.select_first(".page__text").and_then(|x| x.text());

			manga.cover = html
				.select_first(".page__poster img")
				.and_then(|x| x.attr("abs:src"));

			manga.artists = html
				.select_first("ul > li:has(div:contains(Artist))")
				.and_then(|x| x.text())
				.and_then(|x| x.strip_prefix("Artist: ").map(|x| x.to_string()))
				.map(|x| vec![x]);

			manga.authors = html
				.select_first("ul > li:has(div:contains(Writer))")
				.and_then(|x| x.text())
				.and_then(|x| x.strip_prefix("Writer: ").map(|x| x.to_string()))
				.map(|x| vec![x]);

			manga.tags = html.select(".page__tags > a").map(|elements| {
				elements
					.map(|element| element.text().unwrap_or_default())
					.collect::<Vec<String>>()
			});

			let has_tag = |name: &str| {
				manga
					.tags
					.as_ref()
					.is_some_and(|tags| tags.iter().any(|tag| tag.eq_ignore_ascii_case(name)))
			};
			let is_mature = has_tag("mature");
			let is_manga = has_tag("manga");
			manga.content_rating = if is_mature {
				ContentRating::Suggestive
			} else {
				ContentRating::Safe
			};
			// comics read left to right, but manga may not, so those keep the app's default
			manga.viewer = if is_manga {
				Viewer::Unknown
			} else {
				Viewer::LeftToRight
			};

			let status_str = html
				.select_first("ul > li:has(div:contains(Release type))")
				.and_then(|x| x.text())
				.unwrap_or_default();

			manga.status = match status_str
				.strip_prefix("Release type: ")
				.unwrap_or_default()
			{
				"Completed" | "Complete" => MangaStatus::Completed,
				"Ongoing" => MangaStatus::Ongoing,
				_ => MangaStatus::Unknown,
			};
		}

		if needs_chapters {
			let chapter_list = parse_script_json::<ChapterList>(&html, "window.__DATA__")
				.ok_or(error!("No chapter data"))?;

			manga.chapters = Some(
				chapter_list
					.chapters
					.into_iter()
					.map(|chapter| chapter.into_chapter(chapter_list.news_id, &manga.title))
					.collect(),
			);
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let url = format!("{BASE_URL}{}", chapter.key);
		let data = parse_script_json::<ReaderData>(&get_html(&url)?, "window.__DATA__")
			.ok_or(error!("No page data"))?;

		Ok(data
			.images
			.into_iter()
			.map(|image| {
				let url = if image.starts_with('/') {
					format!("{BASE_URL}{image}")
				} else {
					image
				};
				Page {
					content: PageContent::url(url),
					..Default::default()
				}
			})
			.collect())
	}
}

impl ListingProvider for BatCave {
	fn get_manga_list(&self, listing: Listing, page: i32) -> Result<MangaPageResult> {
		get_listing_page(&listing.id, page)
	}
}

/// Returns a page of one of the listings in `LISTING_NAMES`.
fn get_listing_page(id: &str, page: i32) -> Result<MangaPageResult> {
	match id {
		LATEST_LISTING => {
			let (entries, has_next_page) = parse_latest(&get_html(&latest_url(page))?);
			Ok(MangaPageResult {
				entries: entries.into_iter().map(|entry| entry.manga).collect(),
				has_next_page,
			})
		}
		TOP_RATED_LISTING => Ok(parse_manga_list(&post_html(
			&comix_url(page),
			&sort_body("rating", false, "cat_1"),
		)?)),
		JUST_ADDED_LISTING => Ok(parse_manga_list(&get_html(&comix_url(page))?)),
		_ => bail!("Unknown listing: {id}"),
	}
}

impl DynamicFilters for BatCave {
	fn get_dynamic_filters(&self) -> Result<Vec<Filter>> {
		// the site has about 1,800 publishers, so they come from its own filter data
		// instead of filters.json, and search still works without them if that fails
		let Some(site_filters) = get_html(&comix_url(1))
			.ok()
			.and_then(|html| parse_script_json::<SiteFilters>(&html, "window.__XFILTER__"))
		else {
			return Ok(Vec::new());
		};

		let (ids, options): (Vec<_>, Vec<_>) = site_filters
			.filter_items
			.publisher
			.values
			.into_iter()
			.map(|publisher| (publisher.id.to_string().into(), publisher.value.into()))
			.unzip();
		Ok(vec![
			MultiSelectFilter {
				id: "publisher".into(),
				title: Some("Publisher".into()),
				options,
				ids: Some(ids),
				..Default::default()
			}
			.into(),
		])
	}
}

impl ImageRequestProvider for BatCave {
	fn get_image_request(
		&self,
		url: String,
		_context: Option<aidoku::PageContext>,
	) -> Result<Request> {
		if url.contains("batcave.biz") {
			Ok(Request::get(url)?.header("Referer", REFERER))
		} else {
			Ok(Request::get(url)?)
		}
	}
}

impl DeepLinkHandler for BatCave {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(key) = url.strip_prefix(BASE_URL) else {
			return Ok(None);
		};

		// https://batcave.biz/reader/33408/238878
		if let Some((news_id, id)) = key
			.strip_prefix("/reader/")
			.and_then(|ids| ids.split_once('/'))
		{
			let id = id.split(['/', '?', '#']).next().unwrap_or_default();
			let (Ok(news_id), Ok(id)) = (news_id.parse::<i32>(), id.parse::<i32>()) else {
				return Ok(None);
			};
			let key = format!("/reader/{news_id}/{id}");
			// the reader page links back to its comic
			let manga_key = parse_script_json::<ReaderData>(
				&get_html(&format!("{BASE_URL}{key}"))?,
				"window.__DATA__",
			)
			.and_then(|data| data.post_link.strip_prefix(BASE_URL).map(Into::into));
			return Ok(manga_key.map(|manga_key| DeepLinkResult::Chapter { manga_key, key }));
		}

		let Some((id, slug)) = key.strip_prefix('/').and_then(|path| path.split_once('-')) else {
			return Ok(None);
		};
		let Some(slug) = slug.strip_suffix(".html") else {
			return Ok(None);
		};

		if id.is_empty()
			|| slug.is_empty()
			|| !id.bytes().all(|byte| byte.is_ascii_digit())
			|| !slug
				.bytes()
				.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
		{
			return Ok(None);
		}

		Ok(Some(DeepLinkResult::Manga {
			key: key.to_string(),
		}))
	}
}

register_source!(
	BatCave,
	Home,
	ListingProvider,
	DynamicFilters,
	ImageRequestProvider,
	DeepLinkHandler
);
