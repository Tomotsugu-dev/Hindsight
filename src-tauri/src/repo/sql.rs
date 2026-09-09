//! Shared SQL fragments.
//!
//! The activity → group → category chain is walked by reports, AI summaries,
//! chat tools, export and the unclassified-apps list. Written out at every call
//! site it drifted: some places joined `categories` and some did not, the alias
//! for `app_group_members` was `gm` in most places and `m` in one, and the
//! fallback for an unclassified activity was spelled three different ways.
//!
//! These constants are the single definition of the chain. Callers append their
//! own SELECT list and WHERE clause; nothing about filtering belongs here.

/// `FROM` clause resolving an activity to the app group its process belongs to.
///
/// Aliases: `a` = activities, `gm` = app_group_members, `g` = app_groups.
///
/// Both joins are LEFT, so an activity whose process has no group — or whose
/// group is soft-deleted — still comes through, with `g.*` NULL. Callers that
/// need a group identity usually fall back with `COALESCE(g.id, a.process_name)`.
pub const FROM_ACTIVITY_GROUP: &str = "FROM activities a
     LEFT JOIN app_group_members gm
       ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
     LEFT JOIN app_groups g
       ON g.id = gm.group_id AND g.deleted_at IS NULL";

/// [`FROM_ACTIVITY_GROUP`] plus the category that group is assigned to.
///
/// Adds alias `c` = categories. This join is LEFT as well, so `c.id IS NULL`
/// covers all three ways the chain can break: the process has no group, the
/// group is deleted, or `g.category_id` still points at a category that was
/// deleted (a cascade that missed a reference).
///
/// Unclassified activities therefore land on `c.* = NULL`; callers decide what
/// that means — `COALESCE(c.id, 'other')` to bucket them, `c.id IS NULL` to
/// select them.
pub const FROM_ACTIVITY_GROUP_CATEGORY: &str = "FROM activities a
     LEFT JOIN app_group_members gm
       ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
     LEFT JOIN app_groups g
       ON g.id = gm.group_id AND g.deleted_at IS NULL
     LEFT JOIN categories c
       ON c.id = g.category_id AND c.deleted_at IS NULL";

/// `FROM` clause pairing every group membership with its group.
///
/// Aliases: `gm` = app_group_members, `g` = app_groups. Inner join, no
/// `deleted_at` filter — callers wanting live rows add it in WHERE; sync push
/// needs the soft-deleted ones too, to emit tombstones.
pub const FROM_MEMBER_GROUP: &str = "FROM app_group_members gm
     JOIN app_groups g ON g.id = gm.group_id";
