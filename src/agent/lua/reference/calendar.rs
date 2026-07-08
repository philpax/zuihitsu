//! Calendar API reference entries: `calendar.upcoming`, `overdue`, `on`, `recurring`, date
//! construction, and `<date>:*` methods.

use super::super::super::api_doc::{ApiEntry, ApiEntry as AE, ApiType as AT, object};

/// The calendar entries, gated on the `calendar` feature.
pub(super) fn entries() -> Vec<ApiEntry> {
    let upcoming = AE::new("calendar.upcoming")
        .description(
            "Memories with something happening soon (including the next instance of a recurring one), \
             soonest first. Each is a memory handle — read m.name and m.description, or call its \
             methods (m:entries() …) for detail.",
        )
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                "how far ahead to look — a duration string (\"7 days\", \"2 weeks\"), passable directly \
                 in place of the table; defaults to 7 days",
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let overdue = AE::new("calendar.overdue")
        .description(
            "Memories whose dated occurrence has already passed — what slipped by, soonest first. \
             Reach for this alongside calendar.on(today) and calendar.upcoming when someone asks what \
             they should be on top of: those look at today and ahead, so a reminder whose day passed \
             is invisible without it. Recurring occurrences are excluded (their next instance is \
             always ahead). Each is a memory handle.",
        )
        .optional(
            "opts",
            object().optional(
                "within",
                AT::String,
                "how far back to look — a duration string (\"14 days\", \"1 week\"), passable directly \
                 in place of the table; defaults to 14 days",
            ),
            "options",
        )
        .returns(AT::Handle.list());

    let on = AE::new("calendar.on")
        .description(
            "Memories with something happening on a given day. Pass a date object (calendar.today(), \
             calendar.next(\"friday\"), …) or a \"YYYY-MM-DD\" string — the calendar's own return \
             values feed straight back in.",
        )
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
        .description(
            "Today's date as a date object — pass it straight to append as occurred_at, or do \
             arithmetic on it (:add_days, :add_weeks, :add_months, :weekday). A date object prints \
             and concatenates as its \"YYYY-MM-DD\" day (so `Reminder for {calendar.today()}` works), \
             and :to_string() returns that day. Compute dates this way rather than working one out \
             yourself.",
        )
        .returns(AT::Handle);

    let cal_next = AE::new("calendar.next")
        .description(
            "The next date on or after today falling on a weekday, as a date object — \
             calendar.next(\"friday\") is this Friday (today if today is Friday). Use this for \"this \
             Friday\" instead of computing the date.",
        )
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
        .description(
            "A new date shifted by this many weeks — \"the Friday after next\" is \
             calendar.next(\"friday\"):add_weeks(1).",
        )
        .required("weeks", AT::Number, "how many weeks to shift")
        .returns(AT::Handle);

    let date_add_months = AE::new("<date>:add_months")
        .description(
            "A new date shifted by this many months, keeping the day-of-month where it exists and \
             clamping where it does not (31 Jan + 1 month is 28/29 Feb).",
        )
        .required("months", AT::Number, "how many months to shift")
        .returns(AT::Handle);

    let date_weekday = AE::new("<date>:weekday")
        .description("The date's weekday name, e.g. \"Friday\".")
        .returns(AT::String);

    let date_to_string = AE::new("<date>:to_string")
        .description(
            "The date as its \"YYYY-MM-DD\" string. A date also prints and concatenates as this text, \
             so you rarely need to call it explicitly.",
        )
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
