-- @setup
CREATE TABLE sjq1 (n INT, x INT);
CREATE TABLE sjq2 (n INT, y INT);
CREATE TABLE sjq3 (a INT);
INSERT INTO sjq1 VALUES (2, 20), (1, 10);
INSERT INTO sjq2 VALUES (1, 100), (2, 200);
INSERT INTO sjq3 VALUES (1), (2);

-- A bare `ORDER BY` key inside an expression subquery is matched against the sorted
-- input's output columns before the range table, exactly as `findTargetlistEntrySQL92`
-- does — and the match must be **counted**, not taken first. `n` is carried by both
-- sides of the join, so the sorted input names it twice and 18.6 answers `42702` from
-- `scanRTEForColumn`; taking the first match resolved it silently. The join matches
-- nothing, so the key is never evaluated and the eval-time resolver never sees it.
--
-- Two controls identify the cause rather than the shape. The same ambiguity in `WHERE`
-- over the same zero-row join is `42702` on both engines
-- (`subquery_sort_key_ambiguous_output_name_in_where_loud.sql`), so it is the sort-key
-- path and not the plan shape; and an *unbound* name in the same plan shape is `42703`
-- on both (`subquery_sort_key_binds_to_nothing_loud.sql`), so it is the claimant count
-- and not the gate as a whole.
--
-- **The nesting used to be what made this assert anything.** The identical top-level
-- spelling — `SELECT sjq1.x FROM sjq1 JOIN sjq2 ON false ORDER BY n` — answered here
-- against 18.6's `42702` for as long as a level's own lowering resolved its sort keys
-- before the projection existed and so had no claimant set to count. It now carries the
-- same one this walk does, and that spelling is its own fixture
-- (`select/order_by_ambiguous_join_column_over_no_row_loud.sql`); this one stays because
-- the two phases are two derivations of the rule and either can regress alone.
--
-- The write polarity of this fixture's own statement is
-- `crates/pglet/tests/scope_relation_identity.rs` — `@expect-error` compares SQLSTATEs
-- and is green for a refusal raised after the writes.
-- @expect-error: a bare sort key two of the input's columns claim is 42702
-- @test
SELECT a FROM sjq3 WHERE a NOT IN (SELECT sjq1.x FROM sjq1 JOIN sjq2 ON false ORDER BY n);
