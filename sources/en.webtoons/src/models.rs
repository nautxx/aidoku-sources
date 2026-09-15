use aidoku::{
	Chapter,
	alloc::{String, Vec},
	prelude::*,
};
use serde::Deserialize;

use crate::{BASE_URL, IMAGE_URL};

#[derive(Deserialize)]
pub struct EpisodeListResponse {
	pub result: EpisodeList,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeList {
	pub episode_list: Vec<Episode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
	episode_title: String,
	viewer_link: String,
	exposure_date_millis: i64,
	thumbnail: Option<String>,
}

// episode titles from the api can contain html entities
fn unescape(text: &str) -> String {
	let text = text.trim();
	if !text.contains('&') {
		return text.into();
	}
	text.replace("&lt;", "<")
		.replace("&gt;", ">")
		.replace("&quot;", "\"")
		.replace("&#39;", "'")
		.replace("&#039;", "'")
		.replace("&amp;", "&")
}

impl Episode {
	pub fn into_chapter(self, chapter_number: f32) -> Chapter {
		// canvas links carry a reading language that the site's own links leave out
		let key = self.viewer_link.replace("&readingLanguageCode=en", "");
		Chapter {
			url: Some(format!("{BASE_URL}{key}")),
			key,
			title: Some(unescape(&self.episode_title)),
			chapter_number: Some(chapter_number),
			date_uploaded: Some(self.exposure_date_millis / 1000),
			thumbnail: self.thumbnail.map(|path| format!("{IMAGE_URL}{path}")),
			..Default::default()
		}
	}
}
