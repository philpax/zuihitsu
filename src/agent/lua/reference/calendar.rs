//! Calendar API reference entries: `calendar.upcoming`, `overdue`, `on`, `recurring`, date
//! construction, and `<date>:*` methods.

use crate::agent::{
    api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, object},
    body_of,
};

/// The calendar entries, gated on the `calendar` feature.
pub(super) fn entries() -> Vec<ApiEntry> {
    let upcoming = AE::new("calendar.upcoming")
        .description(body_of(include_str!("prose/calendar/upcoming.md")))
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                body_of(include_str!("prose/calendar/upcoming_within.md")),
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let overdue = AE::new("calendar.overdue")
        .description(body_of(include_str!("prose/calendar/overdue.md")))
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                body_of(include_str!("prose/calendar/overdue_within.md")),
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let on = AE::new("calendar.on")
        .description(body_of(include_str!("prose/calendar/on.md")))
        .required(
            "date",
            AT::String,
            "the day — a date object or a \"YYYY-MM-DD\" string",
        )
        .returns(AT::Handle.list());

    let recurring = AE::new("calendar.recurring")
        .description("Memories with a recurring occurrence.")
        .returns(AT::Handle.list());

    let cal_today = AE::new("calendar.today")
        .description(body_of(include_str!("prose/calendar/today.md")))
        .returns(AT::Handle);

    let cal_next = AE::new("calendar.next")
        .description(body_of(include_str!("prose/calendar/next.md")))
        .required("weekday", AT::String, "a weekday name, e.g. \"friday\"")
        .returns(AT::Handle);

    let cal_in_days = AE::new("calendar.in_days")
        .description("The date that many days from today, as a date object (negative goes back).")
        .required("days", AT::Number, "how many days from today")
        .returns(AT::Handle);

    let cal_in_weeks = AE::new("calendar.in_weeks")
        .description("The date that many weeks from today, as a date object.")
        .required("weeks", AT::Number, "how many weeks from today")
        .returns(AT::Handle);

    let cal_date = AE::new("calendar.date")
        .description("Parse an explicit \"YYYY-MM-DD\" into a date object.")
        .required("day", AT::String, "the day as \"YYYY-MM-DD\"")
        .returns(AT::Handle);

    let date_add_days = AE::new("<date>:add_days")
        .description("A new date shifted by this many days (negative goes back).")
        .required("days", AT::Number, "how many days to shift")
        .returns(AT::Handle);

    let date_add_weeks = AE::new("<date>:add_weeks")
        .description(body_of(include_str!("prose/calendar/add_weeks.md")))
        .required("weeks", AT::Number, "how many weeks to shift")
        .returns(AT::Handle);

    let date_add_months = AE::new("<date>:add_months")
        .description(body_of(include_str!("prose/calendar/add_months.md")))
        .required("months", AT::Number, "how many months to shift")
        .returns(AT::Handle);

    let date_weekday = AE::new("<date>:weekday")
        .description("The date's weekday name, e.g. \"Friday\".")
        .returns(AT::String);

    let date_to_string = AE::new("<date>:to_string")
        .description(body_of(include_str!("prose/calendar/to_string.md")))
        .returns(AT::String);

    vec![
        upcoming,
        overdue,
        on,
        recurring,
        cal_today,
        cal_next,
        cal_in_days,
        cal_in_weeks,
        cal_date,
        date_add_days,
        date_add_weeks,
        date_add_months,
        date_weekday,
        date_to_string,
    ]
}
