import type { RunRecord } from "../types/RunRecord.ts";
import type { ScenarioReport } from "../types/ScenarioReport.ts";

/// A single run selected for inspection — the unit the run-scoped views (State, Conversation,
/// Events, Time-travel) operate on. Carries its scenario for context and the run record whose event
/// log the replica folds.
export interface ActiveRun {
  scenario: ScenarioReport;
  run: RunRecord;
}
