-- The bare twin of `update/returning_from_source_column.sql`: a `RETURNING` name only
-- the **`FROM` source** carries, written without a qualifier, over a statement that
-- matches no row.
--
-- One claimant, so it binds — on 18.6 and here. The pair with
-- `returning_from_source_column_ambiguous.sql` (where the target carries the name too,
-- and both engines answer `42702`) is what separates *counting* the claimants from
-- *refusing* every bare name over a `FROM`: a fix that sealed the namespace instead of
-- counting reddens here and leaves its sibling green.
--
-- The `WHERE` matches nothing, so this is also a declared-type fixture — `rbs_s.k` is
-- `bigint` and the target carries no such column, so a build that failed to widen the
-- namespace has no type to declare and must fall back to something wrong.
-- @setup
CREATE TABLE rbs_t (id INT, tag TEXT);
CREATE TABLE rbs_s (id INT, k BIGINT);
INSERT INTO rbs_t VALUES (1, 'before');
INSERT INTO rbs_s VALUES (1, 7);

-- @test
UPDATE rbs_t SET tag = 'hit' FROM rbs_s s WHERE rbs_t.id = 999 AND rbs_t.id = s.id RETURNING k;
