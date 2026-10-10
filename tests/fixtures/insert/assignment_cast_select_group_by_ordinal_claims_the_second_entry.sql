-- The mirror of `assignment_cast_select_group_by_claims_by_position.sql`: the same two
-- equal entries, the ordinal moved. `GROUP BY 2` resolves the **second** entry, which is
-- the one assigned to the `BOOL` column — `42804` on 18.6 where its `GROUP BY 1` twin is
-- accepted.
--
-- It is the fixture a first-match rule fails: scanning the entries for one equal to the
-- substituted key marks entry 1 here and this statement is silently accepted, writing a
-- row 18.6 refuses.
-- @setup
CREATE TABLE acgq (t TEXT, b BOOL);
CREATE TABLE acgqsrc (n INT);
INSERT INTO acgqsrc VALUES (1), (2);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acgq (t, b) SELECT 't', 't' FROM acgqsrc GROUP BY 2;
