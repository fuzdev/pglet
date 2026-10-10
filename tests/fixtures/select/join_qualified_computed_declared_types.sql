-- A **computed expression** over a **qualified** reference into a join whose two sides
-- carry a same-named column of different width. That combination is what makes this
-- fixture discriminating, and each half is load-bearing:
--
-- - *computed*, because a bare or aliased passthrough column is routed straight to
--   `find_projected_column` by `project_output_columns` and never reaches
--   `TypeScope::Input`'s own lookup — the path the qualifier-awareness lives on;
-- - *qualified into a differing sibling*, because a flat by-name lookup over a join's
--   concatenated columns takes the **first** match. `jqc_a.v` is `int2` and `jqc_b.v` is
--   `int8`, so a qualifier-blind resolver declares `from_b` as `int4` (the `int2 + int4`
--   promotion) while the executor sends eight `int8` bytes — a binary client's decode
--   break, not a mislabel.
--
-- The pre-existing join fixtures cannot see it: `select/qualified_join_declared_types.sql`
-- and its two siblings project passthrough columns only, and
-- `select/alias_join_declared_types.sql` joins one table to itself, so both sides carry
-- identical types and no blind lookup can be caught.
--
-- `from_a` is the polarity pin: the narrow side must stay `int4`, so a resolver that
-- overcorrected to "always take the last match" reddens too.
-- @setup
CREATE TABLE jqc_a (v SMALLINT, k INT);
CREATE TABLE jqc_b (v BIGINT, k INT);
INSERT INTO jqc_a VALUES (7, 1);
INSERT INTO jqc_b VALUES (5000000000, 1);

-- @test
SELECT b.v + 0 AS from_b, a.v + 0 AS from_a FROM jqc_a a JOIN jqc_b b ON a.k = b.k;
