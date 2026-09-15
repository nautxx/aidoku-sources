#![no_std]
use aidoku::{
	Chapter, ContentRating, DeepLinkHandler, DeepLinkResult, FilterValue, ImageRequestProvider,
	Listing, ListingProvider, Manga, MangaPageResult, MangaStatus, Page, PageContent, PageContext,
	Result, Source, Viewer,
	alloc::{String, Vec, string::ToString},
	helpers::uri::QueryParameters,
	imports::{
		html::Document,
		net::Request,
		std::{current_date, send_partial_result},
	},
	prelude::*,
};

mod models;
use models::*;

const BASE_URL: &str = "https://www.webtoons.com";
const MOBILE_URL: &str = "https://m.webtoons.com";
// same images as webtoon-phinf, but they load without a referer
const IMAGE_URL: &str = "https://swebtoon-phinf.pstatic.net";
const REFERER: &str = "https://www.webtoons.com/";
const MOBILE_REFERER: &str = "https://m.webtoons.com/";
// skips the age gate and gdpr prompts
const COOKIE: &str = "ageGatePass=true; locale=en; needGDPR=false";

struct Webtoons;

fn get_html(url: &str) -> Result<Document> {
	Ok(Request::get(url)?
		.header("Cookie", COOKIE)
		.header("Referer", REFERER)
		.html()?)
}

fn image_url(url: String) -> String {
	match url.strip_prefix("https://webtoon-phinf.pstatic.net") {
		Some(path) => format!("{IMAGE_URL}{path}"),
		None => url,
	}
}

fn parse_manga_list(html: &Document) -> Vec<Manga> {
	html.select(".webtoon_list li a")
		.map(|els| {
			els.filter_map(|el| {
				let url = el.attr("abs:href")?;
				let key = url.strip_prefix(BASE_URL)?.into();
				Some(Manga {
					key,
					title: el.select_first(".title")?.text()?,
					cover: el
						.select_first("img")
						.and_then(|img| img.attr("abs:src"))
						.map(image_url),
					url: Some(url),
					..Default::default()
				})
			})
			.collect()
		})
		.unwrap_or_default()
}

fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
	query
		.split('&')
		.find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

impl Source for Webtoons {
	fn new() -> Self {
		Self
	}

	fn get_search_manga_list(
		&self,
		query: Option<String>,
		page: i32,
		filters: Vec<FilterValue>,
	) -> Result<MangaPageResult> {
		let mut search_type = "";
		let mut genre = "";
		let mut sort = "MANA";

		for filter in &filters {
			match filter {
				FilterValue::Select { id, value } if id == "type" => search_type = value,
				FilterValue::Select { id, value } if id == "genre" => genre = value,
				FilterValue::Sort { index, .. } => {
					sort = match index {
						1 => "LIKEIT",
						2 => "UPDATE",
						_ => "MANA",
					};
				}
				_ => {}
			}
		}

		let url = match query.as_deref() {
			Some(query) if !query.is_empty() => {
				let mut qs = QueryParameters::new();
				qs.push("keyword", Some(query));
				// searching all types shows a few results of each without pages
				if search_type.is_empty() {
					format!("{BASE_URL}/en/search?{qs}")
				} else {
					qs.push("page", Some(&page.to_string()));
					format!("{BASE_URL}/en/search/{search_type}?{qs}")
				}
			}
			// genre pages list every title at once
			_ if !genre.is_empty() => format!("{BASE_URL}/en/genres/{genre}?sortOrder={sort}"),
			_ => format!("{BASE_URL}/en/ranking/popular"),
		};

		let html = get_html(&url)?;
		Ok(MangaPageResult {
			entries: parse_manga_list(&html),
			has_next_page: html
				.select_first("a.pagination[aria-current=true] + a")
				.is_some(),
		})
	}

	fn get_manga_update(
		&self,
		mut manga: Manga,
		needs_details: bool,
		needs_chapters: bool,
	) -> Result<Manga> {
		let url = format!("{BASE_URL}{}", manga.key);

		if needs_details {
			let html = get_html(&url)?;
			let info = html.select_first(".detail_header .info");

			manga.title = html
				.select_first("h1.subj, h3.subj")
				.and_then(|el| el.text())
				.unwrap_or(manga.title);
			manga.authors = info.as_ref().and_then(|info| {
				// canvas pages link each author, originals list them as text
				let authors: Vec<String> = match info.select("a.author") {
					Some(els) if !els.is_empty() => els.filter_map(|el| el.text()).collect(),
					_ => info
						.select_first(".author_area")?
						.own_text()?
						.split('/')
						.map(|author| author.trim().into())
						.filter(|author: &String| !author.is_empty())
						.collect(),
				};
				(!authors.is_empty()).then_some(authors)
			});
			manga.tags = info
				.as_ref()
				.and_then(|info| info.select(".genre"))
				.map(|els| els.filter_map(|el| el.text()).collect());
			manga.description = html
				.select_first("#_asideDetail p.summary")
				.and_then(|el| el.text());
			manga.status = html
				.select_first("#_asideDetail p.day_info")
				.and_then(|el| el.text())
				.map(|text| {
					if text.contains("COMPLETED") {
						MangaStatus::Completed
					} else if text.contains("UP") || text.contains("EVERY") {
						MangaStatus::Ongoing
					} else {
						MangaStatus::Unknown
					}
				})
				.unwrap_or_default();
			// list covers are portrait posters, og:image is a square crop
			manga.cover = manga.cover.or_else(|| {
				html.select_first("meta[property=og:image]")
					.and_then(|el| el.attr("content"))
			});
			manga.content_rating = ContentRating::Safe;
			manga.viewer = Viewer::Webtoon;
			manga.url = Some(url);

			if needs_chapters {
				send_partial_result(&manga);
			}
		}

		if needs_chapters {
			let title_no = manga
				.key
				.split_once('?')
				.and_then(|(_, query)| query_param(query, "title_no"))
				.ok_or(error!("Missing title number"))?;
			let api_url = if manga.key.starts_with("/en/canvas/") {
				format!(
					"{MOBILE_URL}/api/v1/canvas/{title_no}/episodes?pageSize=99999&readingLanguageCode=en"
				)
			} else {
				format!("{MOBILE_URL}/api/v1/webtoon/{title_no}/episodes?pageSize=99999")
			};
			let episodes = Request::get(api_url)?
				.header("Cookie", COOKIE)
				.header("Referer", MOBILE_REFERER)
				.json_owned::<EpisodeListResponse>()?
				.result
				.episode_list;

			// episode numbers can skip deleted episodes, so number them in order
			manga.chapters = Some(
				episodes
					.into_iter()
					.enumerate()
					.map(|(idx, episode)| episode.into_chapter(idx as f32 + 1.0))
					.rev()
					.collect(),
			);
		}

		Ok(manga)
	}

	fn get_page_list(&self, _manga: Manga, chapter: Chapter) -> Result<Vec<Page>> {
		let html = get_html(&format!("{BASE_URL}{}", chapter.key))?;
		Ok(html
			.select("#_imageList > img")
			.map(|els| {
				els.filter_map(|el| {
					Some(Page {
						content: PageContent::url(image_url(el.attr("data-url")?)),
						..Default::default()
					})
				})
				.collect()
			})
			.unwrap_or_default())
	}
}

impl ListingProvider for Webtoons {
	fn get_manga_list(&self, listing: Listing, _page: i32) -> Result<MangaPageResult> {
		let url = if listing.id == "latest" {
			// 1970-01-01 was a thursday
			const DAYS: [&str; 7] = [
				"thursday",
				"friday",
				"saturday",
				"sunday",
				"monday",
				"tuesday",
				"wednesday",
			];
			let day = DAYS[(current_date() / 86400 % 7) as usize];
			format!("{BASE_URL}/en/originals/{day}?sortOrder=UPDATE")
		} else {
			format!("{BASE_URL}/en/ranking/{}", listing.id)
		};

		Ok(MangaPageResult {
			entries: parse_manga_list(&get_html(&url)?),
			has_next_page: false,
		})
	}
}

impl ImageRequestProvider for Webtoons {
	fn get_image_request(&self, url: String, _context: Option<PageContext>) -> Result<Request> {
		// images are blocked without a referer
		Ok(Request::get(url)?.header("Referer", REFERER))
	}
}

impl DeepLinkHandler for Webtoons {
	fn handle_deep_link(&self, url: String) -> Result<Option<DeepLinkResult>> {
		let Some(path) = [BASE_URL, MOBILE_URL, "https://webtoons.com"]
			.iter()
			.find_map(|base| url.strip_prefix(base))
		else {
			return Ok(None);
		};
		let Some((path, query)) = path.split_once('?') else {
			return Ok(None);
		};
		let Some(title_no) = query_param(query, "title_no") else {
			return Ok(None);
		};

		// https://www.webtoons.com/en/fantasy/tower-of-god/list?title_no=95
		if let Some(series_path) = path.strip_suffix("/list") {
			return Ok(Some(DeepLinkResult::Manga {
				key: format!("{series_path}/list?title_no={title_no}"),
			}));
		}

		// https://www.webtoons.com/en/fantasy/tower-of-god/season-1-ep-0/viewer?title_no=95&episode_no=1
		let (Some(episode_path), Some(episode_no)) = (
			path.strip_suffix("/viewer"),
			query_param(query, "episode_no"),
		) else {
			return Ok(None);
		};
		let Some((series_path, _)) = episode_path.rsplit_once('/') else {
			return Ok(None);
		};

		Ok(Some(DeepLinkResult::Chapter {
			manga_key: format!("{series_path}/list?title_no={title_no}"),
			key: format!("{episode_path}/viewer?title_no={title_no}&episode_no={episode_no}"),
		}))
	}
}

register_source!(
	Webtoons,
	ListingProvider,
	ImageRequestProvider,
	DeepLinkHandler
);
