//! Tests for column resolution and the `42702` ambiguity of bare names in a join scope.

use super::super::columns::{ColumnResolver, JoinColumnMap, SimpleColumns};
use crate::Error;
use crate::types::TypeOid;

#[test]
fn simple_columns_unqualified_join_collision_is_42702() {
    // A join's output names arrive dotted; a bare `num` matching both sides is
    // ambiguous rather than a silent first-match.
    let names = ["a.num", "b.num"];
    let r = SimpleColumns::new(&names);
    let err = r.resolve(None, "num").unwrap_err();
    assert!(matches!(err, Error::AmbiguousColumn { .. }));
    assert_eq!(err.sqlstate(), "42702");
}

#[test]
fn simple_columns_using_merged_bare_resolves() {
    // The prepended bare merged column short-circuits the ambiguity.
    let names = ["num", "a.num", "b.num"];
    let r = SimpleColumns::new(&names);
    assert_eq!(r.resolve(None, "num").unwrap(), 0);
}

#[test]
fn simple_columns_single_suffix_match_resolves() {
    let names = ["a.num", "b.name"];
    let r = SimpleColumns::new(&names);
    assert_eq!(r.resolve(None, "name").unwrap(), 1);
}

#[test]
fn join_column_map_unqualified_collision_is_42702() {
    // A bare column in a JOIN `ON` predicate matching both relations → 42702.
    let mut map = JoinColumnMap::new();
    map.add_column("a", "num", 0, TypeOid::Int4, None);
    map.add_column("b", "num", 1, TypeOid::Int4, None);
    let err = map.resolve(None, "num").unwrap_err();
    assert!(matches!(err, Error::AmbiguousColumn { .. }));
    assert_eq!(err.sqlstate(), "42702");
    // A single match still resolves; a qualified ref picks its side.
    assert_eq!(map.resolve(Some("a"), "num").unwrap(), 0);
}

#[test]
fn simple_columns_valid_qualifier_resolves() {
    // Tier 1: the qualifier IS the effective single-table name → bare match.
    let names = ["v"];
    let r = SimpleColumns::with_table(&names, Some("t"));
    assert_eq!(r.resolve(Some("t"), "v").unwrap(), 0);
}

#[test]
fn simple_columns_quoted_dotted_name_does_not_shadow_an_alias() {
    // A user column quoted into existence as `"t.v"` is byte-identical to the
    // name a join synthesizes for side `t`'s column `v`. In a **single-table**
    // scope the synthesized form cannot occur, so `t.v` must mean alias `t`'s
    // column `v` (index 1) and never the quoted column (index 0) — resolving
    // the dotted name first returned a different column than the qualifier
    // named, with a different declared type.
    //
    // The condition is a precondition on the caller, and giving a *multi*-relation
    // scope to this resolver is what breaks it: `RETURNING` over an `UPDATE … FROM`
    // did that once and answered the wrong relation's column, so it now
    // resolves by `(relation, column)` pair instead (`executor::helpers`'s
    // `project_returning`). A join scope — the one below — used to collide by its own
    // route: `register_input_columns` re-parsed each input column's name for its pair,
    // so the name reaching this resolver was already re-synthesized. Such a scope is
    // built with `ScopeIdentity::Relations` now and resolves through the pairs; this
    // resolver's string tiers are what a scope whose pairs could not be derived keeps.
    let names = ["t.v", "v"];
    let r = SimpleColumns::with_table(&names, Some("t"));
    assert_eq!(r.resolve(Some("t"), "v").unwrap(), 1);

    // …and with a qualifier naming nothing in scope, the quoted column must not
    // answer for it either: that is a `42P01`, exactly as Postgres reports.
    let err = r.resolve(Some("other"), "v").unwrap_err();
    assert_eq!(err.sqlstate(), "42P01");
}

#[test]
fn simple_columns_join_scope_still_resolves_dotted_names() {
    // The counterpart the gate must not break: a multi-table (join) scope has
    // no single effective table (`table: None`) and stores its columns dotted,
    // so a qualified reference resolves through the synthesized name.
    let names = ["a.v", "b.v"];
    let r = SimpleColumns::new(&names);
    assert_eq!(r.resolve(Some("a"), "v").unwrap(), 0);
    assert_eq!(r.resolve(Some("b"), "v").unwrap(), 1);
}

#[test]
fn simple_columns_unknown_qualifier_is_42p01() {
    // Tier 3: a qualifier naming no in-scope table is a `42P01` (missing
    // FROM-clause entry), not a silent loose/suffix match. Even though the
    // bare name `v` exists, `bogus.v` must fail loud — the silent-wrong bug.
    let names = ["v"];
    let r = SimpleColumns::with_table(&names, Some("t"));
    let err = r.resolve(Some("bogus"), "v").unwrap_err();
    assert!(matches!(
        err,
        Error::Catalog(crate::CatalogError::MissingFromClauseEntry { .. })
    ));
    assert_eq!(err.sqlstate(), "42P01");
}

#[test]
fn simple_columns_no_identity_qualifier_loose_matches() {
    // The no-identity resolver (`table: None` — the no-FROM / `VALUES` path
    // and the post-projection `ORDER BY` / aggregate / window key resolvers)
    // has no FROM scope to validate a qualifier against, so a qualified ref
    // resolves by its bare name (this is what keeps `ORDER BY c.id` over a
    // projected `id` working). Only the single-table-identity resolver above
    // raises `42P01`.
    let names = ["v"];
    let r = SimpleColumns::new(&names);
    assert_eq!(r.resolve(Some("bogus"), "v").unwrap(), 0);
}

#[test]
fn join_column_map_unknown_qualifier_is_42p01() {
    // An out-of-scope qualifier on a join scope is a `42P01`, distinct from a
    // bad column (42703) on an in-scope table.
    let mut map = JoinColumnMap::new();
    map.add_column("a", "num", 0, TypeOid::Int4, None);
    map.add_column("b", "num", 1, TypeOid::Int4, None);
    let err = map.resolve(Some("bogus"), "num").unwrap_err();
    assert!(matches!(
        err,
        Error::Catalog(crate::CatalogError::MissingFromClauseEntry { .. })
    ));
    assert_eq!(err.sqlstate(), "42P01");
    // A valid qualified ref across the join still resolves.
    assert_eq!(map.resolve(Some("b"), "num").unwrap(), 1);
}
