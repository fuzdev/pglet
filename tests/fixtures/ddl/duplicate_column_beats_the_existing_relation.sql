-- Adding a gate is a change to statement-level error precedence every time, so every
-- `CREATE TABLE` gate placed above the table's own `42P07` is pinned
-- against it.
-- `MergeAttributes` walks the column list inside `DefineRelation`, ahead of the
-- `heap_create_with_catalog` call whose `get_relname_relid` test reports the existing
-- relation — so the repeat wins. Placed below the `42P07` instead, this statement
-- would answer `42P07` and the gate would be invisible on every existing table.
-- @setup
CREATE TABLE dcber (q INT);
-- @expect-error: the duplicate column outranks the existing relation
-- @expect-message
-- @no-profile-lock: the @test is DDL over a table with no index — the refusal lands before any plan, so index acceleration cannot reach it
-- @test
CREATE TABLE dcber (a INT, a INT);
