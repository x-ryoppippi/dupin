use dupin_core::item::Page;

pub(super) fn page_chip(page: &Page, chip: Option<&str>) -> String {
    let mut title: String = page.title.chars().take(40).collect();
    if page.title.chars().count() > 40 {
        title.pop();
        title.push('…');
    }
    match chip {
        Some(chip) => format!("{chip} › {title}"),
        None => title,
    }
}
