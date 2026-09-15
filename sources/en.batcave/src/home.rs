use crate::{
	BASE_URL, BatCave, JUST_ADDED_LISTING, LATEST_LISTING, LISTING_NAMES, TOP_RATED_LISTING,
	helpers::*,
};
use aidoku::{
	FilterItem, Home, HomeComponent, HomeComponentValue, HomeLayout, HomePartialResult, Listing,
	ListingProvider, Manga, Result,
	alloc::{String, Vec},
	imports::{html::Element, std::send_partial_result},
};

impl Home for BatCave {
	fn get_home(&self) -> Result<HomeLayout> {
		let html = get_html(BASE_URL)?;

		if let Some(section) = html.select_first("section.sect--genres-home") {
			let genres: Vec<FilterItem> = section
				.select("a")
				.map(|links| {
					links
						.filter_map(|link| {
							// genre links look like "Horror (2971)", and the app matches the
							// name against the genre filter
							let text = link.text()?;
							let name = text
								.rsplit_once(" (")
								.map_or(text.as_str(), |(name, _)| name);
							Some(name.into())
						})
						.collect()
				})
				.unwrap_or_default();
			if !genres.is_empty() {
				send(section_title(&section), HomeComponentValue::Filters(genres));
			}
		}

		let (latest, _) = parse_latest(&html);
		if !latest.is_empty() {
			send(
				html.select_first(".sect--latest .sect__title")
					.and_then(|el| el.text()),
				HomeComponentValue::MangaChapterList {
					page_size: Some(4),
					entries: latest,
					listing: Some(listing(LATEST_LISTING)),
				},
			);
		}

		// "hot new releases" and "series worth starting"
		if let Some(sections) = html.select("section.sect--hot") {
			for section in sections {
				let entries: Vec<Manga> = section
					.select(".sect__content > a.grid-item")
					.map(|posters| posters.filter_map(parse_poster).collect())
					.unwrap_or_default();
				if !entries.is_empty() {
					send(section_title(&section), scroller(entries, None));
				}
			}
		}

		// the site's top-rated and just added blocks only have tiny thumbnails,
		// so these rows use the first page of their listings instead
		for (title, id) in [
			("Top-rated comics", TOP_RATED_LISTING),
			("Just added: fresh comics", JUST_ADDED_LISTING),
		] {
			let listing = listing(id);
			if let Ok(result) = self.get_manga_list(listing.clone(), 1)
				&& !result.entries.is_empty()
			{
				send(Some(title.into()), scroller(result.entries, Some(listing)));
			}
		}

		Ok(HomeLayout::default())
	}
}

fn send(title: Option<String>, value: HomeComponentValue) {
	send_partial_result(&HomePartialResult::Component(HomeComponent {
		title,
		value,
		..Default::default()
	}));
}

fn scroller(entries: Vec<Manga>, listing: Option<Listing>) -> HomeComponentValue {
	HomeComponentValue::Scroller {
		entries: entries.into_iter().map(Into::into).collect(),
		listing,
	}
}

fn listing(id: &str) -> Listing {
	let name = LISTING_NAMES
		.iter()
		.find(|(listing_id, _)| *listing_id == id)
		.map_or(id, |(_, name)| name);
	Listing {
		id: id.into(),
		name: name.into(),
		..Default::default()
	}
}

fn section_title(section: &Element) -> Option<String> {
	section
		.select_first(".sect__title")
		.and_then(|el| el.text())
}

fn parse_poster(element: Element) -> Option<Manga> {
	let url = element.attr("abs:href")?;
	Some(Manga {
		key: url.strip_prefix(BASE_URL)?.into(),
		title: element.select_first(".poster__title")?.text()?,
		cover: element
			.select_first("img")
			.and_then(|img| img.attr("abs:data-src")),
		url: Some(url),
		..Default::default()
	})
}
