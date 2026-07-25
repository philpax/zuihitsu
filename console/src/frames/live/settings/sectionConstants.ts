/// The fixed tail of the settings sidebar. The behavioural settings contribute one section per
/// top-level group of the loaded settings tree (compaction, brief, turn, …), derived at render so
/// the list tracks the Rust structs; these two close the list. The maintenance group's own fields
/// render inside the Maintenance section, beside the sweep history and actions they govern, so
/// `maintenance` never appears as a bare behavioural group. The open section rides in the URL as
/// the location's selection segment, so it deep-links and moves with browser back and forward.
export const FIXED_SECTIONS = [
  { id: "maintenance", label: "Maintenance" },
  { id: "environment", label: "Environment" },
] as const;

export type SectionId = string;
