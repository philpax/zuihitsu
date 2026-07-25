import type { Event } from "@zuihitsu/wire/types/Event.ts";
import type { Replica } from "../../lib/replica/replica.ts";
import { formatDate } from "../../lib/format/format.ts";
import { useNavigate } from "../../lib/nav/historyContext.ts";
import { useStream } from "../../lib/nav/useStreamLocation.ts";
import { Eyebrow, Segmented } from "../../components/primitives.tsx";
import { CalendarList } from "./CalendarList.tsx";
import { MonthGrid } from "./MonthGrid.tsx";
import { WeekGrid } from "./WeekGrid.tsx";
import { CALENDAR_MODES, DEFAULT_MODE, asCalendarMode } from "./calendarUtilities.ts";

/// The Calendar view: the agent's dated horizon — its one-off and recurring occurrences, past,
/// present, and future. Three faces ride the URL selection segment: the month grid (the default), the
/// week grid, and the upcoming list. The grids expand recurrences per instance over a visible range
/// they navigate; the list runs forward from now to a horizon. "Now" is the agent's clock at the
/// timeline cursor — the head when following live, the run's end for a finished package — so every face
/// time-travels with a scrub. The grids own their visible range as component state; the mode is the
/// only navigational selection, so browser back and forward walk between the three faces.
export function CalendarView({
  replica,
  events,
  cursor,
}: {
  replica: Replica;
  events: Event[];
  cursor: number;
}) {
  const now = events.reduce(
    (max, event) => (event.seq <= cursor && event.recorded_at > max ? event.recorded_at : max),
    0,
  );
  const navigate = useNavigate();
  const { selection, seq, link } = useStream();
  const mode = asCalendarMode(selection);

  // Switching face is navigation (a pushed history entry). The month grid is the default, so selecting
  // it drops the selection segment to keep the bare `/live/calendar` canonical.
  function selectMode(next: string) {
    navigate(link.view("calendar", { selection: next === DEFAULT_MODE ? undefined : next, seq }));
  }

  return (
    <section>
      <div className="mb-6 flex flex-wrap items-baseline justify-between gap-4">
        <Segmented options={CALENDAR_MODES} value={mode} onChange={selectMode} />
        <Eyebrow>{now > 0 ? `as of ${formatDate(now)}` : "no events yet"}</Eyebrow>
      </div>
      {mode === "list" && <CalendarList replica={replica} now={now} />}
      {mode === "week" && <WeekGrid replica={replica} now={now} />}
      {mode === "month" && <MonthGrid replica={replica} now={now} />}
    </section>
  );
}
