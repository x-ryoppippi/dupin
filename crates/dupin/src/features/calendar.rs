use std::sync::Arc;

use dupin_core::{
    Action, Batch, CommandError, CommandSpec, Config, Item, ItemAction, Payload, Provider, ProviderError,
    QueryEligibility, Request, View,
};
use dupin_mac::privacy::{Access, CalendarAccess, Permission, PermissionStatus};

use super::extensions::Port;
use super::{Bundled, Capability, Feature, PermissionUse};
use crate::commands::{Configure, Providers, id, presentation};

pub(crate) const CALENDAR: &str = "calendar.events";
/// The bundled extension, in extensions/calendar, that lists and searches the events.
pub(crate) const PACKAGE: &str = "calendar";

pub(super) const FEATURE: Feature = Feature {
    configure,
    start,
    permissions: &[PermissionUse {
        capability: Capability::Permission(Permission::Calendars),
        label: "calendar search",
        enabled: |config| config.calendar.enabled,
        request_on_enable: true,
    }],
    bundled: Some(Bundled {
        package: PACKAGE,
        command: CALENDAR,
        enabled: |config| config.calendar.enabled,
    }),
    ..Feature::EMPTY
};

fn configure(config: &Config, commands: &mut Configure) -> Result<(), CommandError> {
    if config.calendar.enabled {
        let calendar = id(CALENDAR);
        let mut spec = CommandSpec::new(
            calendar.clone(),
            30,
            presentation(
                "Search calendar events",
                Some("Calendar"),
                Some("No upcoming events"),
                Some("No matching events"),
                true,
                5.0,
            ),
        )
        .with_query_eligibility(QueryEligibility::Any);
        spec.private_results = true;
        commands.registry.register(spec)?;
        commands.registry.add_keyword(&calendar, "cal")?;
    }
    Ok(())
}

fn start(providers: &mut Providers) {
    let port = providers.port(CALENDAR);
    let access = providers.calendar_access.clone();
    providers.add(CALENDAR, Arc::new(CalendarSearch { access, port }));
}

/// Calendar search: the bundled extension's answer, once macOS allows Dupin to read calendars.
/// The access is Dupin's, not the extension's, so Dupin asks for it on first use and explains
/// a refusal itself, as it does for every feature.
struct CalendarSearch {
    /// `None` in a headless run, whose fixture events need no access.
    access: Option<CalendarAccess>,
    port: Arc<Port>,
}

impl Provider for CalendarSearch {
    fn search(&self, request: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
        self.search_batches(request, &mut |batch| emit(batch.items))
    }

    fn search_batches(&self, request: &Request, emit: &mut dyn FnMut(Batch)) -> Result<(), ProviderError> {
        if request.limit == 0 || request.is_cancelled() {
            return Ok(());
        }
        if let Some(row) = self
            .access
            .as_ref()
            .and_then(|access| access_problem(access.request_on_use()))
        {
            emit(vec![row].into());
            return Ok(());
        }
        match self.port.provider() {
            Some(extension) => extension.search_batches(request, emit),
            None => {
                emit(
                    vec![row(
                        "unavailable",
                        "Calendar search isn't available",
                        "Its extension is off or didn't load; Settings → Extensions says why.".into(),
                        None,
                    )]
                    .into(),
                );
                Ok(())
            }
        }
    }
}

fn access_problem(status: PermissionStatus) -> Option<Item> {
    Some(match status {
        PermissionStatus::Access(Access::Allowed) => return None,
        PermissionStatus::Access(_) => permission_row(),
        PermissionStatus::Unavailable => row(
            "app-bundle",
            "Calendar access is unavailable",
            "Open the packaged Dupin.app to grant access.".into(),
            None,
        ),
        PermissionStatus::Requesting => row(
            "permission-pending",
            "Waiting for Calendar access",
            "Respond to the macOS permission request.".into(),
            None,
        ),
        PermissionStatus::RequestIncomplete => row(
            "permission-incomplete",
            "Calendar access request did not complete",
            "Try Request Access in Dupin Settings → Permissions.".into(),
            None,
        ),
    })
}

fn permission_row() -> Item {
    row(
        "permission",
        "Calendar access required",
        "Allow Full Access in System Settings → Privacy & Security → Calendars.".into(),
        Some((
            "Open Calendar Privacy Settings",
            Permission::Calendars.settings_url().into(),
        )),
    )
}

fn row(id: &str, title: &str, subtitle: String, open: Option<(&str, String)>) -> Item {
    Item {
        payload: Payload::default(),
        id: id.into(),
        view: View::Text {
            title: title.into(),
            subtitle,
        },
        record_usage: false,
        icon: None,
        matched: Vec::new(),
        completion: None,
        score: None,
        primary: open.as_ref().map(|_| "open".into()),
        alternate: None,
        actions: open
            .map(|(label, url)| ItemAction::new("open", label, Action::OpenUrl(url)))
            .into_iter()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Answers;

    impl Provider for Answers {
        fn search(&self, _: &Request, emit: &mut dyn FnMut(Vec<Item>)) -> Result<(), ProviderError> {
            emit(vec![row("event", "Design review", "10:00 · Work".into(), None)]);
            Ok(())
        }
    }

    fn titles(search: &CalendarSearch) -> Vec<String> {
        let mut rows = Vec::new();
        search
            .search(&Request::detached("", 30), &mut |batch| rows.extend(batch))
            .unwrap();
        rows.iter().map(|row| row.display_title().to_owned()).collect()
    }

    #[test]
    fn the_bundled_extension_answers_once_it_has_loaded() {
        let search = CalendarSearch {
            access: None,
            port: Arc::default(),
        };
        assert_eq!(titles(&search), ["Calendar search isn't available"]);
        search.port.set(Some(Arc::new(Answers)));
        assert_eq!(titles(&search), ["Design review"]);
        search.port.set(None);
        assert_eq!(titles(&search), ["Calendar search isn't available"]);
    }

    #[test]
    fn missing_access_is_explained_with_a_way_to_settings() {
        assert!(access_problem(PermissionStatus::Access(Access::Allowed)).is_none());
        let denied = access_problem(PermissionStatus::Access(Access::Denied)).unwrap();
        assert_eq!(denied.display_title(), "Calendar access required");
        assert_eq!(
            denied.action("open"),
            Some(&Action::OpenUrl(Permission::Calendars.settings_url().into()))
        );
        assert_eq!(
            access_problem(PermissionStatus::Requesting)
                .unwrap()
                .display_title(),
            "Waiting for Calendar access"
        );
    }
}
