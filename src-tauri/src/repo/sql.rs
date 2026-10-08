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

// TODO: 统计已经改用 from_stats_category_sql，会套上网站规则；AI 总结、对话工具、导出还在用
// 这个常量，网站规则对它们不生效（ADR-0013 决定 4）。以后合并成一个。
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

/// Finds the category each website counts toward, that is, the website rule that applies
/// to it: its own rule if it has one, otherwise the nearest parent domain's
/// (`live.bilibili.com` without a rule falls under `bilibili.com`).
///
/// Usage: `hosts_sql` gives the websites to look up (one column, no duplicates); the result
/// is `host_rule(host, rule_host, category_id)`, without the websites no rule applies to.
/// Put the returned SQL after `WITH RECURSIVE`.
pub fn matching_rule_host_sql(hosts_sql: &str) -> String {
    format!(
        "hosts(host) AS ({hosts_sql}),
         up(host, cand) AS (
             SELECT host, host FROM hosts
             UNION ALL
             SELECT host, substr(cand, instr(cand, '.') + 1) FROM up
              WHERE instr(cand, '.') > 0
                AND NOT EXISTS (SELECT 1 FROM site_rules r
                                 WHERE r.host = up.cand AND r.browser = '' AND r.device = ''
                                   AND r.deleted_at IS NULL)
         ),
         host_rule(host, rule_host, category_id) AS (
             SELECT up.host, r.host, r.category_id
               FROM up JOIN site_rules r
                 ON r.host = up.cand AND r.browser = '' AND r.device = ''
                AND r.deleted_at IS NULL
         )"
    )
}

/// Generates the leading `WITH RECURSIVE` clause required by statistics queries.
///
/// It collects non-null domains visited in the given date range from `activities`,
/// finds the applicable website rule for each (preferring an exact match, then the
/// nearest parent domain), and defines the `host_rule(host, rule_host, category_id)`
/// CTE for [`from_stats_category_sql`] to join.
///
/// The returned SQL has two `?` parameters, bound in order: start date and end date.
pub fn host_rule_with_sql() -> String {
    format!(
        "WITH RECURSIVE {}",
        matching_rule_host_sql(
            "SELECT DISTINCT url_host FROM activities
              WHERE url_host IS NOT NULL AND local_date >= ? AND local_date <= ?"
        )
    )
}

/// Generates a reusable `FROM … LEFT JOIN …` fragment for statistics queries.
///
/// Using `source` as the input relation, it resolves the “process → app group → category”
/// and “domain → website rule → category” paths, exposing the final category as `c`.
///
/// It contains no `SELECT` or `WHERE` clause; callers must first use
/// [`host_rule_with_sql`] to define the `host_rule` CTE.
pub fn from_stats_category_sql(source: &str) -> String {
    format!(
        "FROM {source} a
         -- Resolve each process to its app group and assigned category.
         LEFT JOIN app_group_members gm
           ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
         LEFT JOIN app_groups g
           ON g.id = gm.group_id AND g.deleted_at IS NULL
         -- Keep the app group's assigned category only if it has not been soft-deleted.
         LEFT JOIN categories ac
           ON ac.id = g.category_id AND ac.deleted_at IS NULL
         -- Join the precomputed matching website rule and its live category.
         LEFT JOIN host_rule hr ON hr.host = a.url_host
         LEFT JOIN categories rc
           ON rc.id = hr.category_id AND rc.deleted_at IS NULL
         -- Resolve the effective category: hidden apps stay hidden; otherwise prefer a live
         -- website-rule category and fall back to the app group's category.
         LEFT JOIN categories c
           ON c.id = CASE WHEN g.category_id = 'hidden' THEN 'hidden'
                          ELSE COALESCE(rc.id, g.category_id) END
          AND c.deleted_at IS NULL"
    )
}
