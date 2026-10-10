-- @setup
CREATE TABLE nska (q INT, a INT, b INT);
INSERT INTO nska VALUES (1, 1, 10), (2, 2, 20), (3, 2, 30);

-- The aggregate spelling of `nested_sort_key_nested_output_name_loud.sql`: an aggregate's
-- output alias nested inside a subquery's `ORDER BY` key. Same rule, and it needs its own
-- fixture because the key rides a `Plan::Aggregate` rather than a `Plan::Project`, so the
-- output names the re-walk pushes come from a different node. It answered `1` here.
-- @expect-error: a nested aggregate alias in a subquery's ORDER BY key is 42703
-- @test
SELECT (SELECT count(*) AS c FROM nska GROUP BY a ORDER BY c + 0 LIMIT 1) AS r;
