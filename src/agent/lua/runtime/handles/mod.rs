//! Handle minting: memory handles, entry handles, link handles, and their rendering helpers.

mod entry;
mod link;
mod memory;
mod search;

pub(crate) use entry::{
    entry_handle_id, entry_selector, make_entry_handle, make_entry_handle_list,
};
pub(crate) use link::{
    get_argument_name, link_target_id, make_link_handle, make_link_handle_list,
    render_neighborhood, render_salient_relations, resolve_exclude,
};
pub(crate) use memory::{
    HandleSelf, handle_id, make_capped_handle_list, make_handle, make_handle_list,
    make_relation_result, readonly_newindex, render_details,
};
pub(crate) use search::{
    SEARCH_QUERY_FIELD, fold_lower, guard_search_taint, guard_search_write, query_names_handle,
};
