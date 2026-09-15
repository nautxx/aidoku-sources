use aidoku::{
	Chapter,
	alloc::{string::String, vec::Vec},
	imports::std::parse_date,
	prelude::*,
};
use serde::Deserialize;

use crate::BASE_URL;

#[derive(Deserialize)]
pub struct ChapterList {
	pub news_id: i32,
	pub chapters: Vec<SingleChapter>,
}

#[derive(Deserialize)]
pub struct SingleChapter {
	date: String,
	id: i32,
	title: String,
}

impl SingleChapter {
	pub fn into_chapter(self, news_id: i32, manga_title: &str) -> Chapter {
		parse_chapter(news_id, self.id, &self.date, &self.title, manga_title)
	}
}

/// Builds a chapter from its reader ids, a "dd.MM.yyyy" date and a title that
/// starts with the comic's title, like "Batman (2016-) #17".
pub fn parse_chapter(news_id: i32, id: i32, date: &str, title: &str, manga_title: &str) -> Chapter {
	let key = format!("/reader/{news_id}/{id}");
	let title = title.strip_prefix(manga_title).unwrap_or(title).trim();
	let chapter_number = title
		.find('#')
		.and_then(|idx| title[idx + 1..].parse::<f32>().ok());
	// a title that's only the issue number, like "#17", already shows as "Chapter 17"
	let is_only_number = title.starts_with('#') && chapter_number.is_some();
	Chapter {
		title: (!is_only_number).then(|| title.into()),
		chapter_number,
		date_uploaded: parse_date(date, "dd.MM.yyyy"),
		url: Some(format!("{BASE_URL}{key}")),
		key,
		..Default::default()
	}
}

#[derive(Deserialize)]
pub struct PageList {
	pub images: Vec<String>,
}
